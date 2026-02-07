//! Frontmatter parsing for task templates.
//!
//! Parses YAML-style `---` delimited frontmatter blocks at the start of
//! template files. Currently supports a single `description` field used
//! to display template summaries in `tsk template list`.

/// Metadata extracted from a template's frontmatter block.
#[derive(Debug)]
pub struct TemplateFrontmatter {
    /// Short description of what the template is for.
    pub description: Option<String>,
}

/// Parses the frontmatter block from template content.
///
/// Looks for a block delimited by `---` lines at the start of the content.
/// Within that block, extracts the `description` key's value. Returns
/// `description: None` if no valid frontmatter block exists or the block
/// does not contain a `description` key.
pub fn parse_frontmatter(content: &str) -> TemplateFrontmatter {
    let body = match find_closing_delimiter(content) {
        Some((opening_len, closing_offset)) => &content[opening_len..closing_offset],
        None => return TemplateFrontmatter { description: None },
    };

    let mut description = None;

    for line in body.lines() {
        if let Some(value) = line.strip_prefix("description:") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                description = Some(trimmed.to_string());
            }
        }
    }

    TemplateFrontmatter { description }
}

/// Strips the frontmatter block from template content, returning the body.
///
/// If the content starts with a valid `---` delimited frontmatter block,
/// returns everything after the closing `---` line. If no valid frontmatter
/// block exists, returns the original content unchanged.
pub fn strip_frontmatter(content: &str) -> &str {
    let (_, closing_offset) = match find_closing_delimiter(content) {
        Some(result) => result,
        None => return content,
    };

    let after_delimiter = &content[closing_offset + 3..];
    after_delimiter
        .strip_prefix("\r\n")
        .or_else(|| after_delimiter.strip_prefix('\n'))
        .unwrap_or(after_delimiter)
}

/// Finds the opening and closing `---` delimiters in frontmatter content.
///
/// Returns `Some((opening_len, closing_offset))` where `opening_len` is the
/// byte length of the opening `---\n` or `---\r\n` line, and `closing_offset`
/// is the byte offset of the closing `---` line relative to the start of
/// the content. Returns `None` if no valid delimiter pair is found.
fn find_closing_delimiter(content: &str) -> Option<(usize, usize)> {
    let opening_len = if content.starts_with("---\n") {
        4
    } else if content.starts_with("---\r\n") {
        5
    } else {
        return None;
    };

    let rest = &content[opening_len..];
    let mut offset = 0;
    for line in rest.lines() {
        if line == "---" {
            return Some((opening_len, opening_len + offset));
        }
        offset += line.len();
        // Account for actual line ending in the source (not just what .lines() returns)
        if rest[offset..].starts_with("\r\n") {
            offset += 2;
        } else if rest[offset..].starts_with('\n') {
            offset += 1;
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_with_description() {
        let content = "---\ndescription: Implement a new feature\n---\n# Body\n";
        let fm = parse_frontmatter(content);
        assert_eq!(fm.description.as_deref(), Some("Implement a new feature"));
    }

    #[test]
    fn test_parse_frontmatter_without_frontmatter() {
        let content = "# Just a template\nNo frontmatter here.\n";
        let fm = parse_frontmatter(content);
        assert!(fm.description.is_none());
    }

    #[test]
    fn test_parse_frontmatter_empty() {
        let content = "---\n---\nBody text\n";
        let fm = parse_frontmatter(content);
        assert!(fm.description.is_none());
    }

    #[test]
    fn test_parse_frontmatter_missing_closing() {
        let content = "---\ndescription: Orphaned\nNo closing delimiter\n";
        let fm = parse_frontmatter(content);
        assert!(fm.description.is_none());
    }

    #[test]
    fn test_parse_frontmatter_no_description_key() {
        let content = "---\nauthor: someone\ntags: misc\n---\n# Body\n";
        let fm = parse_frontmatter(content);
        assert!(fm.description.is_none());
    }

    #[test]
    fn test_strip_frontmatter_with_frontmatter() {
        let content = "---\ndescription: A feature\n---\n# Feature\nBody here.\n";
        let body = strip_frontmatter(content);
        assert_eq!(body, "# Feature\nBody here.\n");
    }

    #[test]
    fn test_strip_frontmatter_without_frontmatter() {
        let content = "# No frontmatter\nJust content.\n";
        let body = strip_frontmatter(content);
        assert_eq!(body, content);
    }

    #[test]
    fn test_strip_frontmatter_empty_frontmatter() {
        let content = "---\n---\nbody";
        let body = strip_frontmatter(content);
        assert_eq!(body, "body");
    }

    #[test]
    fn test_description_replacement_after_strip() {
        let content =
            "---\ndescription: Fix a bug\n---\n# Fix\n{{DESCRIPTION}}\nMore instructions.\n";
        let body = strip_frontmatter(content);
        assert!(body.contains("{{DESCRIPTION}}"));
        let replaced = body.replace("{{DESCRIPTION}}", "Fix the login timeout issue");
        assert!(replaced.contains("Fix the login timeout issue"));
        assert!(!replaced.contains("{{DESCRIPTION}}"));
        assert!(!replaced.contains("description: Fix a bug"));
    }

    #[test]
    fn test_parse_frontmatter_crlf() {
        let content = "---\r\ndescription: A feature\r\n---\r\n# Body\r\n";
        let fm = parse_frontmatter(content);
        assert_eq!(fm.description.as_deref(), Some("A feature"));
    }

    #[test]
    fn test_strip_frontmatter_crlf() {
        let content = "---\r\ndescription: A feature\r\n---\r\n# Feature\r\nBody here.\r\n";
        let body = strip_frontmatter(content);
        assert_eq!(body, "# Feature\r\nBody here.\r\n");
    }

    #[test]
    fn test_strip_frontmatter_missing_closing() {
        let content = "---\ndescription: Orphaned\nNo closing delimiter\n";
        let body = strip_frontmatter(content);
        assert_eq!(body, content);
    }
}
