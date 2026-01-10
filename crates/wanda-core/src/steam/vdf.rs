//! VDF (Valve Data Format) file parsing
//!
//! Steam uses VDF files for configuration (libraryfolders.vdf, appmanifest_*.acf, etc.)

use crate::error::{Result, WandaError};
use std::collections::HashMap;
use std::path::Path;

/// Parsed VDF value - can be a string or a nested object
#[derive(Debug, Clone)]
pub enum VdfValue {
    String(String),
    Object(HashMap<String, VdfValue>),
}

impl VdfValue {
    /// Get as string if this is a string value
    pub fn as_str(&self) -> Option<&str> {
        match self {
            VdfValue::String(s) => Some(s),
            VdfValue::Object(_) => None,
        }
    }

    /// Get as object if this is an object value
    pub fn as_object(&self) -> Option<&HashMap<String, VdfValue>> {
        match self {
            VdfValue::String(_) => None,
            VdfValue::Object(obj) => Some(obj),
        }
    }

    /// Get a nested value by key
    pub fn get(&self, key: &str) -> Option<&VdfValue> {
        self.as_object()?.get(key)
    }

    /// Get a string value by key
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }
}

/// Parse a VDF file from disk
pub fn parse_vdf_file(path: &Path) -> Result<VdfValue> {
    let content = std::fs::read_to_string(path).map_err(|e| WandaError::VdfParseError {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;

    parse_vdf(&content).map_err(|e| WandaError::VdfParseError {
        path: path.to_path_buf(),
        reason: e,
    })
}

/// Parse VDF content from a string
pub fn parse_vdf(content: &str) -> std::result::Result<VdfValue, String> {
    let mut parser = VdfParser::new(content);
    parser.parse_root()
}

struct VdfParser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> VdfParser<'a> {
    fn new(content: &'a str) -> Self {
        Self {
            chars: content.chars().peekable(),
        }
    }

    fn parse_root(&mut self) -> std::result::Result<VdfValue, String> {
        self.skip_whitespace();

        // Root is typically a single key-value pair where value is an object
        let mut root = HashMap::new();

        while self.chars.peek().is_some() {
            self.skip_whitespace();
            if self.chars.peek().is_none() {
                break;
            }

            let key = self.parse_string()?;
            self.skip_whitespace();
            let value = self.parse_value()?;
            root.insert(key, value);
            self.skip_whitespace();
        }

        Ok(VdfValue::Object(root))
    }

    fn parse_value(&mut self) -> std::result::Result<VdfValue, String> {
        self.skip_whitespace();

        match self.chars.peek() {
            Some('{') => self.parse_object(),
            Some('"') => Ok(VdfValue::String(self.parse_string()?)),
            Some(c) => Err(format!("Unexpected character: {}", c)),
            None => Err("Unexpected end of input".to_string()),
        }
    }

    fn parse_object(&mut self) -> std::result::Result<VdfValue, String> {
        self.expect_char('{')?;
        let mut obj = HashMap::new();

        loop {
            self.skip_whitespace();
            match self.chars.peek() {
                Some('}') => {
                    self.chars.next();
                    break;
                }
                Some('"') => {
                    let key = self.parse_string()?;
                    self.skip_whitespace();
                    let value = self.parse_value()?;
                    obj.insert(key, value);
                }
                Some(c) => return Err(format!("Expected '\"' or '}}', got '{}'", c)),
                None => return Err("Unexpected end of input in object".to_string()),
            }
        }

        Ok(VdfValue::Object(obj))
    }

    fn parse_string(&mut self) -> std::result::Result<String, String> {
        self.expect_char('"')?;
        let mut result = String::new();

        loop {
            match self.chars.next() {
                Some('"') => break,
                Some('\\') => {
                    // Handle escape sequences
                    match self.chars.next() {
                        Some('n') => result.push('\n'),
                        Some('t') => result.push('\t'),
                        Some('\\') => result.push('\\'),
                        Some('"') => result.push('"'),
                        Some(c) => {
                            result.push('\\');
                            result.push(c);
                        }
                        None => return Err("Unexpected end of input in escape sequence".to_string()),
                    }
                }
                Some(c) => result.push(c),
                None => return Err("Unexpected end of input in string".to_string()),
            }
        }

        Ok(result)
    }

    fn skip_whitespace(&mut self) {
        while let Some(&c) = self.chars.peek() {
            if c.is_whitespace() {
                self.chars.next();
            } else if c == '/' {
                // Handle // comments
                self.chars.next();
                if self.chars.peek() == Some(&'/') {
                    // Skip until end of line
                    while let Some(&c) = self.chars.peek() {
                        self.chars.next();
                        if c == '\n' {
                            break;
                        }
                    }
                }
            } else {
                break;
            }
        }
    }

    fn expect_char(&mut self, expected: char) -> std::result::Result<(), String> {
        match self.chars.next() {
            Some(c) if c == expected => Ok(()),
            Some(c) => Err(format!("Expected '{}', got '{}'", expected, c)),
            None => Err(format!("Expected '{}', got end of input", expected)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_vdf() {
        let vdf = r#"
        "libraryfolders"
        {
            "0"
            {
                "path"      "/home/user/.local/share/Steam"
                "label"     ""
            }
        }
        "#;

        let result = parse_vdf(vdf).unwrap();
        let folders = result.get("libraryfolders").unwrap();
        let folder0 = folders.get("0").unwrap();
        assert_eq!(
            folder0.get_str("path"),
            Some("/home/user/.local/share/Steam")
        );
    }

    #[test]
    fn test_parse_app_manifest() {
        let acf = r#"
        "AppState"
        {
            "appid"     "292030"
            "name"      "The Witcher 3: Wild Hunt"
            "installdir"    "The Witcher 3 Wild Hunt"
            "SizeOnDisk"    "50000000000"
        }
        "#;

        let result = parse_vdf(acf).unwrap();
        let app_state = result.get("AppState").unwrap();
        assert_eq!(app_state.get_str("appid"), Some("292030"));
        assert_eq!(
            app_state.get_str("name"),
            Some("The Witcher 3: Wild Hunt")
        );
    }
}
