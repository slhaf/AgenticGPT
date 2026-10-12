use std::path::Path;

#[derive(Clone, Debug)]
pub(super) enum Atom {
    Literal(char),
    Star,
    Recursive,
}

pub(super) fn command_tokens(pattern: &str) -> Result<Vec<Vec<Atom>>, String> {
    if pattern.trim().is_empty() {
        return Err("command_policy_pattern_empty".to_string());
    }
    let mut tokens = Vec::new();
    let mut token = Vec::new();
    let mut active = false;
    let mut quote = None;
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), ch) => token.push(Atom::Literal(ch)),
            (_, '\\') => {
                let escaped = chars
                    .next()
                    .ok_or_else(|| "command_policy_pattern_escape_incomplete".to_string())?;
                if quote == Some('"') && !matches!(escaped, '"' | '\\' | '$' | '`' | '\n') {
                    token.push(Atom::Literal('\\'));
                }
                if escaped != '\n' {
                    token.push(Atom::Literal(escaped));
                }
                active = true;
            }
            (Some(_), ch) => token.push(Atom::Literal(ch)),
            (None, '\'' | '"') => {
                quote = Some(ch);
                active = true;
            }
            (None, ch) if ch.is_whitespace() => {
                if active {
                    tokens.push(std::mem::take(&mut token));
                    active = false;
                }
            }
            (None, '*') => {
                token.push(Atom::Star);
                active = true;
            }
            (None, ch) => {
                token.push(Atom::Literal(ch));
                active = true;
            }
        }
    }
    if quote.is_some() {
        return Err("command_policy_pattern_quote_incomplete".to_string());
    }
    if active {
        tokens.push(token);
    }
    for (index, token) in tokens.iter().enumerate() {
        if matches!(token.as_slice(), [Atom::Star, Atom::Star]) && index + 1 != tokens.len() {
            return Err("command_policy_recursive_wildcard_must_be_final".to_string());
        }
        if token
            .windows(2)
            .any(|pair| matches!(pair, [Atom::Star, Atom::Star]))
            && !matches!(token.as_slice(), [Atom::Star, Atom::Star])
        {
            return Err("command_policy_recursive_wildcard_must_be_whole_token".to_string());
        }
    }
    Ok(tokens)
}

pub(super) fn command_matches(pattern: &str, program: &str, args: &[String]) -> bool {
    if pattern.trim() == "*" {
        return true;
    }
    let Ok(tokens) = command_tokens(pattern) else {
        return false;
    };
    let recursive = tokens
        .last()
        .is_some_and(|token| matches!(token.as_slice(), [Atom::Star, Atom::Star]));
    let count = tokens.len() - usize::from(recursive);
    let argv = std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>();
    if (recursive && argv.len() < count) || (!recursive && argv.len() != count) {
        return false;
    }
    tokens
        .iter()
        .take(count)
        .zip(argv)
        .all(|(token, value)| glob_matches(token, value, false))
}

fn glob_matches(pattern: &[Atom], value: &str, path: bool) -> bool {
    // Literal argv can contain large scripts or escaped output. Comparing them
    // directly avoids a quadratic table that delays admission beyond waitSeconds.
    if pattern.iter().all(|atom| matches!(atom, Atom::Literal(_))) {
        return pattern
            .iter()
            .filter_map(|atom| match atom {
                Atom::Literal(ch) => Some(*ch),
                _ => None,
            })
            .eq(value.chars());
    }
    let chars = value.chars().collect::<Vec<_>>();
    let mut previous = vec![false; chars.len() + 1];
    previous[0] = true;
    for atom in pattern {
        let mut next = vec![false; chars.len() + 1];
        match atom {
            Atom::Literal(expected) => {
                for index in 1..=chars.len() {
                    next[index] = previous[index - 1] && chars[index - 1] == *expected;
                }
            }
            Atom::Star | Atom::Recursive => {
                next[0] = previous[0];
                for index in 1..=chars.len() {
                    let consumes =
                        !path || matches!(atom, Atom::Recursive) || chars[index - 1] != '/';
                    next[index] = previous[index] || (next[index - 1] && consumes);
                }
            }
        }
        previous = next;
    }
    previous[chars.len()]
}

pub(super) fn path_pattern(pattern: &str) -> Result<Vec<Atom>, String> {
    if pattern.trim() == "*" {
        return Ok(vec![Atom::Recursive]);
    }
    if pattern.is_empty() {
        return Err("path_policy_pattern_empty".to_string());
    }
    let expanded = crate::exec::expand_pathbuf(Path::new(pattern))
        .map_err(|error| format!("path_policy_pattern_expansion_failed: {error}"))?;
    if !expanded.is_absolute() {
        return Err("path_policy_pattern_must_be_absolute".to_string());
    }
    let raw = expanded
        .to_str()
        .ok_or_else(|| "path_policy_pattern_not_utf8".to_string())?;
    let mut atoms = Vec::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                atoms.push(Atom::Literal(chars.next().ok_or_else(|| {
                    "path_policy_pattern_escape_incomplete".to_string()
                })?))
            }
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                atoms.push(Atom::Recursive);
            }
            '*' => atoms.push(Atom::Star),
            ch => atoms.push(Atom::Literal(ch)),
        }
    }
    // Resolve only literal components, preserving escaped stars and backslashes.
    let wildcard = atoms
        .iter()
        .position(|atom| !matches!(atom, Atom::Literal(_)));
    let literal = |atoms: &[Atom]| {
        atoms
            .iter()
            .filter_map(|atom| match atom {
                Atom::Literal(ch) => Some(*ch),
                _ => None,
            })
            .collect::<String>()
    };
    if let Some(index) = wildcard {
        let boundary = atoms[..index]
            .iter()
            .rposition(|atom| matches!(atom, Atom::Literal('/')))
            .unwrap_or(0);
        let prefix = if boundary == 0 {
            "/".to_string()
        } else {
            literal(&atoms[..boundary])
        };
        let root = crate::exec::canonicalize_existing_or_parent(Path::new(&prefix))
            .map_err(|error| format!("path_policy_pattern_resolution_failed: {error}"))?;
        let mut resolved = root
            .to_str()
            .ok_or_else(|| "path_policy_pattern_not_utf8".to_string())?
            .trim_end_matches('/')
            .chars()
            .map(Atom::Literal)
            .collect::<Vec<_>>();
        resolved.extend_from_slice(&atoms[boundary..]);
        Ok(resolved)
    } else {
        let root = crate::exec::canonicalize_existing_or_parent(Path::new(&literal(&atoms)))
            .map_err(|error| format!("path_policy_pattern_resolution_failed: {error}"))?;
        Ok(root
            .to_str()
            .ok_or_else(|| "path_policy_pattern_not_utf8".to_string())?
            .chars()
            .map(Atom::Literal)
            .collect())
    }
}

pub(super) fn path_matches(pattern: &[Atom], value: &str) -> bool {
    if glob_matches(pattern, value, true) {
        return true;
    }
    // A trailing recursive subtree wildcard also includes the directory itself.
    if matches!(pattern.last(), Some(Atom::Recursive))
        && matches!(
            pattern.get(pattern.len().saturating_sub(2)),
            Some(Atom::Literal('/'))
        )
    {
        return glob_matches(
            &pattern[..pattern.len() - 2],
            value.trim_end_matches('/'),
            true,
        );
    }
    false
}
