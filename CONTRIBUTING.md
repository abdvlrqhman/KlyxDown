# Contributing to KlyxDown

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Before you open a pull request

1. Set up the project as described in the [README](README.md#build-from-source).
2. Keep changes focused: one fix or feature per pull request.
3. Make sure the checks pass locally:

   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   npm run check
   ```

4. Describe what changed and how you tested it.

## License of contributions

KlyxDown is licensed under the GPL-3.0-or-later, and Spacie also offers it under commercial licenses (see the [README](README.md#license)). So that both stay possible, by submitting a contribution you confirm that you wrote it (or have the right to submit it) and you grant Spacie a perpetual, worldwide, royalty-free license to use, modify and distribute your contribution under the GPL-3.0-or-later and under Spacie's commercial licenses. You keep the copyright to your work.

## Security issues

Please do not open a public issue for security problems. Email [support@spacie.net](mailto:support@spacie.net) instead.
