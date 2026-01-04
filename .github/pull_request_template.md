# Pull request

Write a short description of what has been changed and what has been accomplished, e.g., which issues are resolved by merging this pull request. 

Ideally pull request should be focussed on introducing a single features or fixing single bugs to avoid large pull requests that eventually go stale. If you have a larger feature in
mind it makes sense to discuss it with other maintainers beforehand.

Mark the pull request as a draft when it is not yet ready for review. Optionally, add a check list of changes that still should be done.

## Checklist

 - Ensure that `cargo clippy` passes without warnings, and that the code is properly formatted with `cargo +nightly fmt`.
 - The tests pass locally (also of the GUI and mCRL2 workspaces). Although the CI will also test many other configurations so some test failures can be expected.
 - The branch can be merged into `main` without merge conflicts.
