# Contributing

Thank you for looking at the code. Bug reports, questions and pull requests
are welcome.

## Before you start

- **Bugs and questions**: open an issue with what you did, what you expected
  and what happened. For a security problem, do not open an issue: use
  GitHub's private vulnerability reporting (see [SECURITY.md](SECURITY.md)).
- **Larger changes** (a new storage tier, a new provider, a change of the API
  or of the store format): open an issue first so the design can be agreed
  before the code is written. The store format is documented in
  [docs/STORAGE.md](docs/STORAGE.md) and its rules apply to every change
  that touches the write path.

## Setting up

[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) explains how to run the three
development servers, the tests and the coverage measurement. Once per
clone, enable the local hooks, which run the same guard as the CI before
each commit and push:

```sh
git config core.hooksPath .githooks
```

## What a pull request needs

- Tests at the level where the behaviour is visible: unit, integration or
  end to end. The CI enforces **90 % line coverage** per Rust crate and on
  the web suite; a change that lowers a crate below the bar does not merge.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo test --workspace` green; for the web, `npm run test:coverage`
  and `npx tsc --noEmit` in `web/`.
- No new dependency when a few lines do the job; a new dependency says why
  in the pull request.
- Every source file carries the SPDX header of its neighbours.
- The CI runs on branches named `feat/**` and `sec/**` and on pull
  requests, so name your branch accordingly if you want a run before the
  pull request exists.

## Licensing of contributions

The project is dual-licensed (AGPL-3.0-or-later or a commercial licence,
see [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md)). By submitting a pull
request you license your contribution to Mediasoft & Cie S.A. under both
tracks, so the project can keep offering the commercial option. You keep
the copyright of your work.
