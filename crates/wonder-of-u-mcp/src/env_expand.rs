use wonder_of_u_core::{Result, WonderError};

fn is_var_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_var_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

/// Expands `$VAR` and `${VAR}` references in an MCP environment value.
pub fn expand_env_value(
    input: &str,
    context: &str,
    strict: bool,
    getenv: impl Fn(&str) -> Option<String>,
) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.char_indices().peekable();

    while let Some((_, ch)) = chars.next() {
        if ch != '$' {
            output.push(ch);
            continue;
        }

        let Some(&(next_index, next)) = chars.peek() else {
            output.push('$');
            continue;
        };

        if next == '{' {
            chars.next();
            let name_start = next_index + next.len_utf8();
            let mut name_end = None;
            for (index, candidate) in chars.by_ref() {
                if candidate == '}' {
                    name_end = Some(index);
                    break;
                }
            }
            let Some(name_end) = name_end else {
                return Err(WonderError::validation(format!(
                    "unclosed `${{` in {context}"
                )));
            };
            let name = &input[name_start..name_end];
            if name.is_empty() || !name.chars().all(is_var_continue) {
                return Err(WonderError::validation(format!(
                    "invalid environment variable reference `${{{name}}}` in {context}"
                )));
            }
            push_env_value(&mut output, name, context, strict, &getenv)?;
            continue;
        }

        if !is_var_start(next) {
            output.push('$');
            continue;
        }

        let name_start = next_index;
        let mut name_end = next_index;
        while let Some(&(index, candidate)) = chars.peek() {
            if !is_var_continue(candidate) {
                break;
            }
            chars.next();
            name_end = index + candidate.len_utf8();
        }
        let name = &input[name_start..name_end];
        push_env_value(&mut output, name, context, strict, &getenv)?;
    }

    Ok(output)
}

fn push_env_value(
    output: &mut String,
    name: &str,
    context: &str,
    strict: bool,
    getenv: &impl Fn(&str) -> Option<String>,
) -> Result<()> {
    match getenv(name) {
        Some(value) => output.push_str(&value),
        None if strict => {
            return Err(WonderError::validation(format!(
                "{context} references unset environment variable `{name}`"
            )));
        }
        None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::expand_env_value;

    fn getenv(name: &str) -> Option<String> {
        match name {
            "TOKEN" => Some("secret".into()),
            "HOME" => Some("/home/demo".into()),
            _ => None,
        }
    }

    #[test]
    fn env_expand_replaces_bare_and_braced_variables() {
        assert_eq!(
            expand_env_value("Bearer $TOKEN", "test", true, getenv).expect("expand"),
            "Bearer secret"
        );
        assert_eq!(
            expand_env_value("${HOME}/bin", "test", true, getenv).expect("expand"),
            "/home/demo/bin"
        );
    }

    #[test]
    fn env_expand_missing_var_strict_errors() {
        let error =
            expand_env_value("$MISSING", "test env", true, getenv).expect_err("missing var");
        assert!(error.to_string().contains("MISSING"));
    }

    #[test]
    fn env_expand_missing_var_lenient_expands_to_empty() {
        assert_eq!(
            expand_env_value("x${MISSING}y", "test", false, getenv).expect("expand"),
            "xy"
        );
    }

    #[test]
    fn env_expand_unclosed_brace_errors() {
        let error = expand_env_value("${TOKEN", "test env", true, getenv).expect_err("unclosed");
        assert!(error.to_string().contains("unclosed"));
    }

    #[test]
    fn env_expand_leaves_non_variable_dollar_literal() {
        assert_eq!(
            expand_env_value("cost $5", "test", true, getenv).expect("expand"),
            "cost $5"
        );
    }
}
