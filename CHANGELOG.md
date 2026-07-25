# Changelog

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
