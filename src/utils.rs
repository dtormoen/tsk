/// Utilities for string manipulation and formatting
/// Sanitizes a string to be safe for use in git branch names.
/// Replaces spaces and special characters with hyphens, removes consecutive hyphens,
/// and ensures the result is lowercase.
pub fn sanitize_for_branch_name(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_for_branch_name() {
        assert_eq!(sanitize_for_branch_name("Hello World"), "hello-world");
        assert_eq!(sanitize_for_branch_name("Fix: Bug #123"), "fix-bug-123");
        assert_eq!(
            sanitize_for_branch_name("feat/new-feature"),
            "feat-new-feature"
        );
        assert_eq!(sanitize_for_branch_name("UPPERCASE"), "uppercase");
        assert_eq!(
            sanitize_for_branch_name("multiple   spaces"),
            "multiple-spaces"
        );
        assert_eq!(
            sanitize_for_branch_name("special!@#$%^chars"),
            "special-chars"
        );
        assert_eq!(
            sanitize_for_branch_name("dots.and_underscores"),
            "dots.and_underscores"
        );
        assert_eq!(
            sanitize_for_branch_name("--multiple--dashes--"),
            "multiple-dashes"
        );
        assert_eq!(
            sanitize_for_branch_name("Fix bug: implement user auth"),
            "fix-bug-implement-user-auth"
        );
        assert_eq!(
            sanitize_for_branch_name("Add feature (user profile)"),
            "add-feature-user-profile"
        );
        assert_eq!(
            sanitize_for_branch_name("Refactor: Clean up database queries"),
            "refactor-clean-up-database-queries"
        );
    }
}
