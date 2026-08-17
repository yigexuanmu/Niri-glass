//! 光标特效配置桥接（任务 2 填实）。
//!
//! 把 niri-config 的 `CursorEffect` 运行时配置项灌进 `CursorEffectState`。
//! niri-config 的 schema（`CursorEffect`/`CursorEffectPart`/`ColorRgb`/`ClickTrigger`）
//! 位于 `niri-config/src/misc.rs`，本文件仅做 config → state 的投影。
