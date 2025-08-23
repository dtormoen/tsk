# Bug Fix Request
There is a bug that you need to fix. The following description of the bug was provided:

{{DESCRIPTION}}

## Bug Fix Best Practices
While working on the bug fix, keep in mind the following:
- Leave the code better than when you got it.
- Avoid changes extraneous to the bug.
- Try to add a minimal test case that proves the bug is fixed, but don't make substantial changes to testing approach if the fix cannot be easily tested.

## Final Steps
After you fix the bug, please do the following steps:
- Add or update tests to make sure the fix is working as expected
- Run `just precommit` and make sure it passes. Fix any issues that it finds, even if unrelated to your changes
- Make sure documentation is up to date. Keep it simple, but ensure documentation is accurate with the current state of the code
- Commit your changes following the Conventional Commits specification with a descriptive summary of the changes
