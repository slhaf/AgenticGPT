use std::fmt;
use std::fs;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SecretReferenceError {
    InvalidReference,
    PlaintextRejected,
    Unavailable,
    InvalidValue,
}

impl fmt::Display for SecretReferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidReference => "secret_reference_invalid",
            Self::PlaintextRejected => "secret_reference_plaintext_rejected",
            Self::Unavailable => "secret_unavailable",
            Self::InvalidValue => "secret_value_invalid",
        })
    }
}

impl std::error::Error for SecretReferenceError {}

pub(crate) fn validate_reference(reference: &str) -> Result<(), SecretReferenceError> {
    if reference.chars().any(char::is_control) {
        return Err(SecretReferenceError::InvalidReference);
    }
    if let Some(name) = reference.strip_prefix("env:") {
        if name.is_empty()
            || !name.bytes().enumerate().all(|(index, byte)| {
                byte == b'_'
                    || byte.is_ascii_alphanumeric() && (index > 0 || byte.is_ascii_alphabetic())
            })
        {
            return Err(SecretReferenceError::InvalidReference);
        }
        return Ok(());
    }
    if let Some(path) = reference.strip_prefix("file:") {
        if path.trim().is_empty() {
            return Err(SecretReferenceError::InvalidReference);
        }
        return Ok(());
    }
    Err(SecretReferenceError::PlaintextRejected)
}

pub(crate) fn resolve_reference(reference: &str) -> Result<String, SecretReferenceError> {
    validate_reference(reference)?;
    let value = if let Some(name) = reference.strip_prefix("env:") {
        std::env::var(name).map_err(|_| SecretReferenceError::Unavailable)?
    } else if let Some(path) = reference.strip_prefix("file:") {
        fs::read_to_string(path)
            .map_err(|_| SecretReferenceError::Unavailable)?
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    } else {
        return Err(SecretReferenceError::PlaintextRejected);
    };
    if value.trim().is_empty() {
        return Err(SecretReferenceError::Unavailable);
    }
    if value.chars().any(char::is_control) {
        return Err(SecretReferenceError::InvalidValue);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn validates_only_supported_reference_forms() {
        assert!(validate_reference("env:AGENTIC_SECRET").is_ok());
        assert!(validate_reference("file:/run/secrets/agentic").is_ok());
        assert_eq!(
            validate_reference("env:1BAD").unwrap_err(),
            SecretReferenceError::InvalidReference
        );
        assert_eq!(
            validate_reference("plaintext").unwrap_err(),
            SecretReferenceError::PlaintextRejected
        );
        assert_eq!(
            validate_reference("file:/tmp/secret\nvalue").unwrap_err(),
            SecretReferenceError::InvalidReference
        );
    }

    #[test]
    fn resolves_environment_secret() {
        let name = format!("AGENTIC_SECRET_TEST_{}", Uuid::new_v4().simple());
        std::env::set_var(&name, "environment-value");
        assert_eq!(
            resolve_reference(&format!("env:{name}")).unwrap(),
            "environment-value"
        );
        std::env::remove_var(&name);
    }

    #[test]
    fn resolves_file_secret_and_rejects_empty_or_control_values() {
        let root = std::env::temp_dir().join(format!("agentic-secret-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();

        let valid = root.join("valid");
        fs::write(&valid, "secret-value\r\n\n").unwrap();
        assert_eq!(
            resolve_reference(&format!("file:{}", valid.display())).unwrap(),
            "secret-value"
        );

        let empty = root.join("empty");
        fs::write(&empty, "\r\n\n").unwrap();
        assert_eq!(
            resolve_reference(&format!("file:{}", empty.display())).unwrap_err(),
            SecretReferenceError::Unavailable
        );

        let invalid = root.join("invalid");
        fs::write(&invalid, "secret\nvalue\n").unwrap();
        assert_eq!(
            resolve_reference(&format!("file:{}", invalid.display())).unwrap_err(),
            SecretReferenceError::InvalidValue
        );

        let missing = root.join("missing");
        assert_eq!(
            resolve_reference(&format!("file:{}", missing.display())).unwrap_err(),
            SecretReferenceError::Unavailable
        );
        let _ = fs::remove_dir_all(root);
    }
}
