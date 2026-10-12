use tree_sitter::{Node, Parser};

/// A statically recognized command. Incomplete entries contain only the literal
/// leading argv prefix and must not be treated as complete for path preflight.
#[derive(Debug)]
pub(crate) struct LiteralCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) start_byte: usize,
    pub(crate) end_byte: usize,
    /// Whether this command's full argv is literal and has no unsupported syntax.
    pub(crate) complete: bool,
}

/// Commands recognized in the script and whether the entire script is supported.
#[derive(Debug)]
pub(crate) struct Extraction {
    pub(crate) commands: Vec<LiteralCommand>,
    pub(crate) complete: bool,
}

/// Extract simple literal invocations from supported Bash command chains.
///
/// Unsupported constructs still contribute recognizable nested commands so policy
/// can preserve deny precedence; callers may preflight only complete invocations.
pub(crate) fn extract_literal_commands(script: &str) -> Extraction {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .is_err()
    {
        return Extraction {
            commands: Vec::new(),
            complete: false,
        };
    }
    let Some(tree) = parser.parse(script, None) else {
        return Extraction {
            commands: Vec::new(),
            complete: false,
        };
    };

    let root = tree.root_node();
    let crlf_offsets = crlf_line_break_offsets(script, root);
    let mut extraction = Extraction {
        commands: Vec::new(),
        complete: !root.has_error() && crlf_offsets.is_empty(),
    };
    extraction.complete &= visit_node(root, script, &mut extraction.commands);

    if !crlf_offsets.is_empty() {
        for invocation in &mut extraction.commands {
            if crlf_offsets.iter().any(|offset| {
                *offset < invocation.end_byte && offset.saturating_add(3) > invocation.start_byte
            }) {
                invocation.complete = false;
            }
        }

        if let Some(normalized) = bash_crlf_analysis_view(script, &crlf_offsets) {
            if let Some(tree) = parser.parse(&normalized, None) {
                let root = tree.root_node();
                let mut normalized_commands = Vec::new();
                let _ = visit_node(root, &normalized, &mut normalized_commands);
                for mut invocation in normalized_commands {
                    let follows_mismatched_newline = crlf_offsets
                        .iter()
                        .any(|offset| invocation.start_byte >= offset.saturating_add(3));
                    let already_found = extraction
                        .commands
                        .iter()
                        .any(|existing| existing.start_byte == invocation.start_byte);
                    if follows_mismatched_newline && !already_found {
                        invocation.complete = false;
                        extraction.commands.push(invocation);
                    }
                }
            }
        }
        extraction.complete = false;
    }

    if extraction.commands.is_empty() {
        extraction.complete = false;
    }
    extraction
}

fn crlf_line_break_offsets(script: &str, root: Node<'_>) -> Vec<usize> {
    let mut literal_ranges = Vec::new();
    collect_literal_ranges(root, &mut literal_ranges);
    let bytes = script.as_bytes();
    let mut offsets = Vec::new();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] == b'\\' && bytes[index + 1] == b'\r' && bytes[index + 2] == b'\n' {
            let range_index = literal_ranges.partition_point(|range| range.end <= index);
            if !literal_ranges
                .get(range_index)
                .is_some_and(|range| range.contains(&index))
            {
                offsets.push(index);
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    offsets
}

fn collect_literal_ranges(node: Node<'_>, ranges: &mut Vec<std::ops::Range<usize>>) {
    if matches!(
        node.kind(),
        "raw_string" | "ansi_c_string" | "string_content" | "comment" | "heredoc_body"
    ) {
        ranges.push(node.byte_range());
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_literal_ranges(child, ranges);
    }
}

fn bash_crlf_analysis_view(script: &str, offsets: &[usize]) -> Option<String> {
    let mut bytes = script.as_bytes().to_vec();
    for offset in offsets {
        let sequence = bytes.get_mut(*offset..offset.saturating_add(3))?;
        sequence.copy_from_slice(b" \n ");
    }
    String::from_utf8(bytes).ok()
}

fn visit_node(node: Node<'_>, source: &str, commands: &mut Vec<LiteralCommand>) -> bool {
    match node.kind() {
        "program" => visit_sequence(node, source, commands, &[";", "\n", "\r\n"]),
        "list" => visit_sequence(node, source, commands, &["&&", "||"]),
        "pipeline" => visit_sequence(node, source, commands, &["|"]),
        "comment" => true,
        "command" => {
            let (command, complete) = extract_command(node, source);
            if let Some(command) = command {
                commands.push(command);
            }
            collect_nested_commands(node, source, commands);
            complete
        }
        "declaration_command" | "unset_command" => {
            if let Some(command) = extract_builtin_command(node, source) {
                commands.push(command);
            }
            collect_nested_commands(node, source, commands);
            false
        }
        _ => {
            collect_nested_commands(node, source, commands);
            false
        }
    }
}

fn visit_sequence(
    node: Node<'_>,
    source: &str,
    commands: &mut Vec<LiteralCommand>,
    allowed_separators: &[&str],
) -> bool {
    let mut complete = !node.has_error();
    let sequence_command_start = commands.len();
    let mut previous_end = node.start_byte();
    for index in 0..node.child_count() {
        let Some(child) = node.child(index) else {
            complete = false;
            continue;
        };
        let gap = source
            .get(previous_end..child.start_byte())
            .unwrap_or_default();
        // A continuation can join '#' to the preceding word, while Tree-sitter
        // incorrectly treats the rest of that line as a comment. Keep this
        // unsupported boundary incomplete rather than silently dropping code.
        let joins_comment =
            child.kind() == "comment" && !gap.is_empty() && literal_word_gap(gap) == Some(false);
        let gap_is_plain = is_sequence_separator(gap) && !joins_comment;
        if !gap_is_plain {
            complete = false;
            if joins_comment {
                if let Some(invocation) = commands.last_mut() {
                    invocation.complete = false;
                }
            }
        }

        if child.is_named() {
            let first_command = commands.len();
            if !visit_node(child, source, commands) {
                complete = false;
            }
            if !gap_is_plain {
                for invocation in &mut commands[first_command..] {
                    invocation.complete = false;
                }
            }
            if has_escaped_whitespace(gap) && first_command < commands.len() {
                commands.remove(first_command);
            }
        } else {
            if !child
                .utf8_text(source.as_bytes())
                .is_ok_and(|separator| allowed_separators.contains(&separator))
            {
                complete = false;
            }
            if !gap_is_plain && commands.len() > sequence_command_start {
                if let Some(invocation) = commands.last_mut() {
                    invocation.complete = false;
                }
            }
        }
        previous_end = child.end_byte();
    }

    let trailing = source
        .get(previous_end..node.end_byte())
        .unwrap_or_default();
    if !is_sequence_separator(trailing) {
        complete = false;
        if commands.len() > sequence_command_start {
            if let Some(invocation) = commands.last_mut() {
                invocation.complete = false;
            }
        }
    }
    complete
}

fn is_sequence_separator(gap: &str) -> bool {
    gap.chars()
        .all(|character| matches!(character, ' ' | '\t' | '\n'))
        || literal_word_gap(gap).is_some()
}

/// Tree-sitter treats Bash line continuations as extras, even within one word.
/// Preserve whether the remaining gap separates argv words or joins fragments.
fn literal_word_gap(gap: &str) -> Option<bool> {
    let mut bytes = gap.as_bytes().iter().copied();
    let mut separated = false;
    while let Some(byte) = bytes.next() {
        match byte {
            b' ' | b'\t' => separated = true,
            b'\\' if bytes.next() == Some(b'\n') => {}
            _ => return None,
        }
    }
    Some(separated)
}

fn has_escaped_whitespace(gap: &str) -> bool {
    gap.as_bytes().windows(2).any(|bytes| {
        bytes[0] == b'\\' && matches!(bytes[1], b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c')
    })
}

fn extract_command(node: Node<'_>, source: &str) -> (Option<LiteralCommand>, bool) {
    let mut program: Option<String> = None;
    let mut program_is_reliable = true;
    let mut args: Vec<String> = Vec::new();
    let mut args_are_prefix = true;
    let mut complete = !node.has_error();
    let mut previous_end = node.start_byte();

    for index in 0..node.child_count() {
        let Some(child) = node.child(index) else {
            complete = false;
            continue;
        };
        let field = node.field_name_for_child(index as u32);
        let gap = source
            .get(previous_end..child.start_byte())
            .unwrap_or_default();
        let word_gap = literal_word_gap(gap);
        if word_gap.is_none() {
            complete = false;
            match field {
                Some("name") => {
                    program_is_reliable = false;
                }
                Some("argument") => {
                    let begins_with_separator = gap
                        .chars()
                        .next()
                        .is_some_and(|character| matches!(character, ' ' | '\t'));
                    if !begins_with_separator {
                        let had_prior_args = !args.is_empty();
                        args.clear();
                        if !had_prior_args {
                            program_is_reliable = false;
                            program = None;
                        }
                    }
                    args_are_prefix = false;
                }
                _ => {}
            }
        }

        match field {
            Some("name") => {
                if let Some(value) = literal_word(child, source) {
                    if program_is_reliable {
                        program = Some(value);
                    }
                } else {
                    complete = false;
                }
            }
            Some("argument") => {
                if let Some(value) = literal_word(child, source) {
                    if args_are_prefix {
                        if word_gap == Some(false) && !gap.is_empty() {
                            if let Some(argument) = args.last_mut() {
                                argument.push_str(&value);
                            } else if let Some(program) = program.as_mut() {
                                program.push_str(&value);
                            }
                        } else {
                            args.push(value);
                        }
                    }
                } else {
                    if word_gap == Some(false) && !gap.is_empty() && args_are_prefix {
                        if args.pop().is_none() {
                            program = None;
                        }
                    }
                    args_are_prefix = false;
                    complete = false;
                }
            }
            Some("redirect") => complete = false,
            _ => complete = false,
        }
        previous_end = child.end_byte();
    }

    let trailing = source
        .get(previous_end..node.end_byte())
        .unwrap_or_default();
    if literal_word_gap(trailing).is_none() {
        complete = false;
    }

    let Some(program) = program else {
        return (None, false);
    };
    (
        Some(LiteralCommand {
            program,
            args,
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            complete,
        }),
        complete,
    )
}

/// Special builtin statements remain unsupported for Allow, but their literal
/// names and reliable argv prefixes must still participate in Deny matching.
fn extract_builtin_command(node: Node<'_>, source: &str) -> Option<LiteralCommand> {
    let keyword = node.child(0)?;
    let keyword_text = keyword.utf8_text(source.as_bytes()).ok()?;
    if !matches!(
        keyword_text,
        "declare" | "typeset" | "export" | "readonly" | "local" | "unset" | "unsetenv"
    ) {
        return None;
    }
    let mut program = keyword_text.to_string();
    let mut args: Vec<String> = Vec::new();
    let mut previous_end = keyword.end_byte();
    for index in 1..node.child_count() {
        let child = node.child(index)?;
        let gap = source.get(previous_end..child.start_byte())?;
        let Some(separated) = literal_word_gap(gap) else {
            if !gap.starts_with([' ', '\t']) && args.pop().is_none() {
                return None;
            }
            break;
        };
        let value = if child.kind() == "variable_name" {
            Some(child.utf8_text(source.as_bytes()).ok()?.to_string())
        } else {
            literal_word(child, source)
        };
        let Some(value) = value else {
            if !separated && !gap.is_empty() && args.pop().is_none() {
                return None;
            }
            break;
        };
        if !separated && !gap.is_empty() {
            if let Some(argument) = args.last_mut() {
                argument.push_str(&value);
            } else {
                program.push_str(&value);
            }
        } else {
            args.push(value);
        }
        previous_end = child.end_byte();
    }
    Some(LiteralCommand {
        program,
        args,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        complete: false,
    })
}

fn collect_nested_commands(node: Node<'_>, source: &str, commands: &mut Vec<LiteralCommand>) {
    for index in 0..node.child_count() {
        let Some(child) = node.child(index) else {
            continue;
        };
        if !child.is_named() {
            continue;
        }
        if matches!(
            child.kind(),
            "command" | "declaration_command" | "unset_command"
        ) {
            let _ = visit_node(child, source, commands);
        } else {
            collect_nested_commands(child, source, commands);
        }
    }
}

fn literal_word(node: Node<'_>, source: &str) -> Option<String> {
    if node.has_error() && node.kind() != "string" {
        return None;
    }
    match node.kind() {
        "command_name" => {
            let mut cursor = node.walk();
            let mut named_children = node.named_children(&mut cursor);
            let child = named_children.next()?;
            if named_children.next().is_some() {
                return None;
            }
            literal_word(child, source)
        }
        "word" => decode_unquoted(node.utf8_text(source.as_bytes()).ok()?),
        "raw_string" => {
            let text = node.utf8_text(source.as_bytes()).ok()?;
            let contents = text.strip_prefix('\'')?.strip_suffix('\'')?;
            Some(contents.to_string())
        }
        "string" => decode_double_quoted_word(node.utf8_text(source.as_bytes()).ok()?),
        "number" if node.child_count() == 0 => {
            Some(node.utf8_text(source.as_bytes()).ok()?.to_string())
        }
        "concatenation" => {
            let mut output = String::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if !child.is_named() {
                    return None;
                }
                output.push_str(&literal_word(child, source)?);
            }
            Some(output)
        }
        _ => None,
    }
}

fn decode_unquoted(text: &str) -> Option<String> {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.next() {
                Some('\n') => {}
                Some(escaped) => output.push(escaped),
                None => return None,
            }
        } else if matches!(character, '*' | '?' | '[' | ']' | '{' | '}' | '~') {
            // These unquoted characters may expand before argv reaches the matcher.
            return None;
        } else if matches!(character, '\r' | '\n') {
            // Bash treats LF as a command terminator; CR is not a word separator.
            return None;
        } else {
            output.push(character);
        }
    }
    Some(output)
}

fn decode_double_quoted_word(text: &str) -> Option<String> {
    let contents = text.strip_prefix('"')?.strip_suffix('"')?;
    decode_double_quoted(contents)
}

fn decode_double_quoted(text: &str) -> Option<String> {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.next() {
                Some('\n') => {}
                Some(escaped @ ('\\' | '"' | '$' | '`')) => output.push(escaped),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => return None,
            }
        } else {
            match character {
                '$' if starts_double_quoted_expansion(chars.clone()) => return None,
                '$' => output.push('$'),
                '`' => return None,
                _ => output.push(character),
            }
        }
    }
    Some(output)
}

fn starts_double_quoted_expansion(mut chars: std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    // Bash removes unescaped line continuations before recognizing expansions.
    // A dollar followed by a continuation and HOME is still "$HOME".
    loop {
        match chars.next() {
            Some('\\') if chars.next() == Some('\n') => continue,
            Some(next) => {
                return next.is_ascii_alphanumeric()
                    || matches!(
                        next,
                        '_' | '*' | '@' | '#' | '?' | '-' | '$' | '!' | '{' | '(' | '['
                    );
            }
            None => return false,
        }
    }
}
