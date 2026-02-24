---
description: Implement a new feature or change
---

# Feature Request
You need to implement a new feature or change. The details of the feature are:

{{PROMPT}}

## Coding Best Practices
- Write documentation following rustdoc best practices.
- Avoid writing unit tests with side effects.
- Avoid mocks unless they are needed to avoid unit tests with side effects.
- Improving existing tests is better than adding new tests

## Final Steps
After you implement the change, please do the following steps:
- Add or update tests to make sure the feature is working as expected
- Run `just precommit` and make sure it passes. Fix any issues that it finds, even if unrelated to your changes
- Make sure documentation is up to date. Keep it simple, but ensure documentation is accurate with the current state of the code
- Commit your changes following the Conventional Commits specification with a descriptive summary of the changes
