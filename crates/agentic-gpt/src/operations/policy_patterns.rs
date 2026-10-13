#[derive(Clone, Debug)]
pub(super) enum Atom {
    Literal(char),
    Star,
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
        .all(|(token, value)| glob_matches(token, value))
}

fn glob_matches(pattern: &[Atom], value: &str) -> bool {
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
            Atom::Star => {
                next[0] = previous[0];
                for index in 1..=chars.len() {
                    next[index] = previous[index] || next[index - 1];
                }
            }
        }
        previous = next;
    }
    previous[chars.len()]
}
