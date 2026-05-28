use crate::{HotIndexError, Result};

pub fn normalize_aptos_address(input: &str) -> Result<String> {
    let trimmed = input.trim();
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);

    if hex.is_empty() {
        return Err(HotIndexError::Address("address is empty".to_string()));
    }

    if hex.len() > 64 {
        return Err(HotIndexError::Address(format!(
            "address has {} hex chars, maximum is 64",
            hex.len()
        )));
    }

    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(HotIndexError::Address(format!(
            "address contains non-hex characters: {input}"
        )));
    }

    Ok(format!("0x{:0>64}", hex.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::normalize_aptos_address;

    #[test]
    fn normalizes_short_address() {
        assert_eq!(
            normalize_aptos_address("0xabc").unwrap(),
            "0x0000000000000000000000000000000000000000000000000000000000000abc"
        );
    }

    #[test]
    fn normalizes_full_uppercase_address() {
        let input = "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(
            normalize_aptos_address(input).unwrap(),
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
    }

    #[test]
    fn rejects_invalid_address() {
        assert!(normalize_aptos_address("0xnot_hex").is_err());
    }

    #[test]
    fn rejects_overlong_address() {
        let address = format!("0x{}", "a".repeat(65));
        assert!(normalize_aptos_address(&address).is_err());
    }
}
