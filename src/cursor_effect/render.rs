//! 光标特效 GLES 渲染（任务 4 填实）。
//!
//! 复用 `render_helpers::ShaderRenderElement` + 程序化 SDF soft-edge，
//! 加法混合（`glBlendFunc(GL_ONE, GL_ONE)`，对应 BASpark `globalCompositeOperation="lighter"`）。
