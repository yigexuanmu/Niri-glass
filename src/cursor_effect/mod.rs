//! 光标特效（Cursor Effects）模块。
//!
//! 以 BASpark（MIT, Canvas2D 矢量管线）为蓝本的 1:1 复刻：跟随鼠标的矢量粒子特效
//!（点击爆裂 + 拖尾 + 程序化光标点），在 niri 合成器内部用 GLES 绘制。
//!
//! 设计文档：`docs/superpowers/specs/cursor-effects-design.md`
//! 实现计划：`docs/superpowers/plans/cursor-effects-implementation.md`

pub mod config;
pub mod render;
pub mod state;

pub use state::CursorEffectState;
