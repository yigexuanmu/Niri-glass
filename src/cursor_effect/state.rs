//! 光标特效纯逻辑状态机（任务 1 完整实现，本文件任务 0 仅占位骨架）。
//!
//! BASpark `index.html` 的 `MouseSpark` 物理/动画逻辑 1:1 翻译为纯 Rust，
//! 无 GLES、无 niri 渲染依赖，可在单元测试中独立驱动。

use std::collections::VecDeque;
use std::time::Instant;

/// 光标特效运行时状态（任务 1 填实全部字段与方法）。
#[derive(Debug)]
pub struct CursorEffectState {
    /// 总开关（config `enable` + 运行时 toggle 共同决定）。
    pub enabled: bool,
    // 任务 1 将在此扩充 color / scale / opacity / waves / sparks / trail 等字段。
    // 占位字段保证任务 0 骨架可编译；任务 1 重写为本结构体的完整版本。
    _placeholder: (),
}

impl Default for CursorEffectState {
    fn default() -> Self {
        Self {
            enabled: true,
            _placeholder: (),
        }
    }
}

impl CursorEffectState {
    pub fn new() -> Self {
        Self::default()
    }
}
