# Changelog

## [0.5.0](https://github.com/kerryhatcher/ferrisbar/compare/v0.4.0...v0.5.0) (2026-08-12)


### Features

* append a repo-cost segment to the daily chip ([463f99c](https://github.com/kerryhatcher/ferrisbar/commit/463f99c0929f51f1ce48d29e2e08951236233d92))

## [0.4.0](https://github.com/kerryhatcher/ferrisbar/compare/v0.3.0...v0.4.0) (2026-08-12)


### Features

* add [analytics] config section ([2a2a992](https://github.com/kerryhatcher/ferrisbar/commit/2a2a9925a01cb033c7132c213e3019102a89a631))
* add budget windows for session, daily, weekly, monthly, and 5h block ([#15](https://github.com/kerryhatcher/ferrisbar/issues/15)) ([f99216a](https://github.com/kerryhatcher/ferrisbar/commit/f99216a7c0b77f61d7a3e491d3d2242f746de720))
* add optional analytics feature and redb dependency ([3621acb](https://github.com/kerryhatcher/ferrisbar/commit/3621acb26818a3e2db214d4e37327662fb36a8e4))
* add the analytics report engine (filtering, JSON/CSV rendering) ([b01e48a](https://github.com/kerryhatcher/ferrisbar/commit/b01e48a775d80d19544e634d790531438b74e602))
* add the analytics store (Sink, Row, key encoding) ([b9e3b58](https://github.com/kerryhatcher/ferrisbar/commit/b9e3b58a58fad54f1a2f75482dde39edc9ce1c53))
* capture cwd on parsed transcript records ([83c4367](https://github.com/kerryhatcher/ferrisbar/commit/83c4367850b3c67f69f6bd0945d1f49437a5be7c))
* feed the analytics store from the existing daily-cost refresh ([8bd95eb](https://github.com/kerryhatcher/ferrisbar/commit/8bd95eb047ba4563aaf242efae10d5d17d3dfac6))
* resolve repo identity from a working directory's git remote ([116b37a](https://github.com/kerryhatcher/ferrisbar/commit/116b37a52605c524e38105e945255246736bbf59))
* session and daily cost chips ([#14](https://github.com/kerryhatcher/ferrisbar/issues/14)) ([056ad56](https://github.com/kerryhatcher/ferrisbar/commit/056ad56e593e87aa3d26613630ea413dcb555a78))
* wire up the ferrisbar report subcommand ([2da8311](https://github.com/kerryhatcher/ferrisbar/commit/2da8311fe0663f6d4eaaf2f8984428956cded193))


### Bug Fixes

* analytics record placement + allow sweep ([a752ff2](https://github.com/kerryhatcher/ferrisbar/commit/a752ff235d5b8d30508703a3d8190ce30ef7c856))
* correct plan doc errors found in pre-flight review ([52f0186](https://github.com/kerryhatcher/ferrisbar/commit/52f0186d9804e10d0419f2c54f310bab79ee12ee))
* escape backslashes when embedding paths in test JSON fixtures ([b33f44a](https://github.com/kerryhatcher/ferrisbar/commit/b33f44a544c149f9533a80357d1517113e43664a))
* resolve worktree identity correctly and fix visibility warnings ([5270884](https://github.com/kerryhatcher/ferrisbar/commit/5270884546306a089710f5e81ff71d003e32c92c))
* windows CI test isolation + saturating_add + stale comment ([65a0495](https://github.com/kerryhatcher/ferrisbar/commit/65a0495fe35173a0dcfe9f86498d60441f41fcbc))

## [0.3.0](https://github.com/kerryhatcher/ferrisbar/compare/v0.2.0...v0.3.0) (2026-07-26)


### Features

* config file, JSONL logging, and display customization ([8b0d226](https://github.com/kerryhatcher/ferrisbar/commit/8b0d226cd3a03c1cf1af1a8220e85deb97e69d32))

## [0.2.0](https://github.com/kerryhatcher/ferrisbar/compare/v0.1.0...v0.2.0) (2026-07-25)


### Features

* add context_bar module for context-usage rendering ([e22c8b8](https://github.com/kerryhatcher/ferrisbar/commit/e22c8b858115cbf431a5e12317fd2b9a393fab50))
* add layout module for statusline composition ([a469f0e](https://github.com/kerryhatcher/ferrisbar/commit/a469f0e69b7c49c95224380dd6cd8a642e0717f6))
* add mystatusline binary that prints Hello World ([d8f0a44](https://github.com/kerryhatcher/ferrisbar/commit/d8f0a44cb4a97a328b1709d0fd1b1f850b3cad7f))
* add payload module for stdin JSON parsing ([ee0cf6b](https://github.com/kerryhatcher/ferrisbar/commit/ee0cf6be01b6e1b9f167146e0520f9d2aba2537f))
* add setup module for updating statusLine settings ([51d9c4a](https://github.com/kerryhatcher/ferrisbar/commit/51d9c4a84305afbeb10049bb8467f8eacb169dca))
* add todo module for active in-progress task lookup ([3084a9b](https://github.com/kerryhatcher/ferrisbar/commit/3084a9bdbdb26c15611a875c9ac6e943a5d39a43))
* wire setup subcommand into main, document it in README ([28244b0](https://github.com/kerryhatcher/ferrisbar/commit/28244b00dc85c940129341ac4ae73bd478d0d763))
* wire statusline modules into main, replace hello-world output ([b4a0422](https://github.com/kerryhatcher/ferrisbar/commit/b4a0422b4f215f2d1fa286363d5a19a2d3a14a88))


### Bug Fixes

* degrade unexpectedly-typed JSON fields to None instead of failing the whole payload ([82fc74b](https://github.com/kerryhatcher/ferrisbar/commit/82fc74bbad630a25687e7ff9126ed441ac79b893))
* honor CLAUDE_CONFIG_DIR in setup, fail loudly on unresolvable config dir ([95a5c42](https://github.com/kerryhatcher/ferrisbar/commit/95a5c42c90b4f515158c407472ee9e3a28652826))
* remove unnecessary [lib] section from Cargo.toml ([d9869ac](https://github.com/kerryhatcher/ferrisbar/commit/d9869acc83529b7163b58cc21c8fdb0a3850b6a6))
* use is_none_or instead of map_or(true, ...) in todo.rs ([5ef1a02](https://github.com/kerryhatcher/ferrisbar/commit/5ef1a022fb85be9a37fed83c495043f719cbf747))
