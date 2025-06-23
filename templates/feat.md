# Feature Request
You need to implement a new feature or change. The description of the feature is:

{{DESCRIPTION}}

## Coding Best Practices
- Write documentation following rustdoc best practices.
- Avoid writing unit tests with side effects.
- Avoid mocks unless they are needed to avoid unit tests with side effects.

## Final Steps
After you implement the change, please do the following steps:
- Add or update tests to make sure the feature is working as expected
- Run `just precommit` to make sure files are formatted correctly and tests pass
- Make sure documentation is up to date. Keep it simple, but ensure documentation is accurate with the current state of the code
- Commit your changes following the Conventional Commits specification with a descriptive summary of the changes
