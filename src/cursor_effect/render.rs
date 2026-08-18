//! 光标特效 GLES 渲染层（Cursor Effects Rendering）。1:1 从 BASpark `index.html`
//! 的 `_updateWaves` / `_updateSparks` / `_updateTrail` 源码翻译到 GLES SDF。
//!
//! 复用仓库既有的 `ShaderRenderElement`（与 `BorderRenderElement` 同模板），把
//! `CursorEffectState` 中 waves/rings/sparks/trail 的几何按 BASpark 的绘制顺序
//! 逐一翻译为 SDF primitive（实心圆 / 圆弧描边 / 三角形 / 胶囊线段）。
//!
//! 坐标空间：quad 用 `niri_v_coords`（归一化 `[k, k+1]`），shader 通过 `input_to_geo`
//! 矩阵（= `Mat3::from_scale(area_size)`，等价 `border.rs` 的 `geo_loc=0` 情形）转到
//! 几何像素空间（Canvas 风格左上原点）。所有 `u_center`/`u_p*` 都用这个空间。
//! 没有 `input_to_geo`，SDF 会全部命中，整个 quad 亮成方块（即原本的 bug 1）。

use std::collections::HashMap;
use std::rc::Rc;

use glam::{Mat3, Vec2};

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    ffi, GlesError, GlesFrame, GlesRenderer, GlesTexture, Uniform,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::gpu_span_location;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::cursor_effect::glyphs;
use crate::render_helpers::renderer::{AsGlesFrame as _, NiriRenderer};
use crate::render_helpers::shader_element::ShaderRenderElement;
use crate::render_helpers::shaders::{mat3_uniform, ProgramType, Shaders};

use crate::cursor_effect::state::{self, CursorEffectState};

/// 一个光标特效粒子基元的渲染参数。
#[derive(Debug, Clone, Copy)]
struct Params {
    mode: f32,
    color: [f32; 3],
    center: [f32; 2],
    radius: f32,
    inner_w: f32,
    aa: f32,
    a0: f32,
    a1: f32,
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    trail_a0: f32,
    trail_a1: f32,
    alpha: f32,
    glyph: f32,
    rot: f32,
}

/// 光标特效里一个粒子基元的渲染元素（与 `BorderRenderElement` 同模板）。
#[derive(Debug, Clone)]
pub struct CursorEffectElement {
    inner: ShaderRenderElement,
}

impl CursorEffectElement {
    /// BASpark `index.html:390` `_strokeRingSegment` 包装：圆弧描边（圆角 cap）。
    /// `radius_px` / `inner_w` / `alpha` / `color` bytes proportion。
    fn build(
        size: Size<f64, Logical>,
        loc: Point<f64, Logical>,
        params: Params,
        scale: f32,
        trail_pts: &[[f32; 2]],
        atlas: Option<&GlesTexture>,
    ) -> Self {
        let area = Vec2::new(size.w as f32, size.h as f32);
        // 几何充满整个 quad，`geo_loc=0`，所以 `input_to_geo = Scale(area)`，
        // `coords_geo = niri_v_coords * area`。
        let input_to_geo = Mat3::from_scale(area);
        let geo_size = area;

        let mut uniforms: Vec<Uniform> = vec![
            mat3_uniform("input_to_geo", input_to_geo),
            Uniform::new("geo_size", geo_size.to_array()),
            Uniform::new("u_mode", params.mode),
            Uniform::new("u_color", params.color),
            Uniform::new("u_center", params.center),
            Uniform::new("u_radius", params.radius),
            Uniform::new("u_inner_w", params.inner_w),
            Uniform::new("u_aa", params.aa),
            Uniform::new("u_a0", params.a0),
            Uniform::new("u_a1", params.a1),
            Uniform::new("u_p0", params.p0),
            Uniform::new("u_p1", params.p1),
            Uniform::new("u_p2", params.p2),
            Uniform::new("u_trail_a0", params.trail_a0),
            Uniform::new("u_trail_a1", params.trail_a1),
            // 根因修复（trail polyline）：单 quad 包络整条折线；shader 内逐段最小
            // distance field 一次画完，消除逐段 butt-cap 连接点 halo 归零的细缝。
            // 仅 mode 3 (trail_polyline) 填充 `trail_pts`；其余 mode 传空切片。
            Uniform::new("u_trail_count", trail_pts.len() as f32),
            // mode 4 (glyph)：字形索引 + 旋转（Rust 侧按 ~200ms 节拍闪烁，相位打散）。
            Uniform::new("u_glyph", params.glyph),
            Uniform::new("u_rot", params.rot),
        ];
        for (i, pt) in trail_pts.iter().enumerate() {
            uniforms.push(Uniform::new(format!("u_trail_pts[{}]", i), *pt));
        }

        let textures = atlas
            .map(|tex| HashMap::from([(String::from("cursor_glyph_atlas"), tex.clone())]))
            .unwrap_or_default();

        let mut inner = ShaderRenderElement::empty(ProgramType::CursorEffect, Kind::Unspecified);
        inner.update(
            size,
            None,
            scale,
            params.alpha,
            Rc::from(uniforms),
            textures,
        );
        inner = inner.with_location(loc);
        Self { inner }
    }

    /// 把全局逻辑坐标减去 output 左上 → 输出局部坐标。
    fn local(center_global: Point<f64, Logical>, output_loc: Point<f64, Logical>) -> (f64, f64) {
        (center_global.x - output_loc.x, center_global.y - output_loc.y)
    }

    /// BASpark `index.html:402-411` `updateFilledCircle`：实心圆。
    pub fn filled_circle(
        center_global: Point<f64, Logical>,
        radius_px: f32,
        color: [f32; 3],
        alpha: f32,
        output_loc: Point<f64, Logical>,
        scale: f32,
        aa: f32,
    ) -> Self {
        let (lx, ly) = Self::local(center_global, output_loc);
        let r = radius_px.max(0.5) as f64;
        // AA 抗锯齿：quad 必须在形状外再包住 aa 半径，否则 shader 的 1px 软边
        // 在 quad 边界（圆的四个轴向点）被裁剪成硬边，出现尖角/锯齿。
        let rc = r + aa as f64;
        let size = Size::from((rc * 2., rc * 2.));
        let loc = Point::from((lx - rc, ly - rc));
        Self::build(
            size,
            loc,
            Params {
                mode: 0.0,
                color,
                center: [rc as f32, rc as f32],
                radius: r as f32,
                inner_w: 0.0,
                aa,
                a0: 0.0,
                a1: 0.0,
                p0: [0.0, 0.0],
                p1: [0.0, 0.0],
                p2: [0.0, 0.0],
                trail_a0: 0.0,
                trail_a1: 0.0,
                alpha,
                glyph: 0.0,
                rot: 0.0,
            },
            scale,
            &[],
            None,
        )
    }

    /// BASpark `index.html:390-397` `_strokeRingSegment`：单段圆弧描边。
    /// radius=圆弧半径、inner_w=描边宽度、a0/a1=弧起止角度（弧度）。
    pub fn ring_segment(
        center_global: Point<f64, Logical>,
        radius_px: f32,
        inner_w: f32,
        a0: f32,
        a1: f32,
        color: [f32; 3],
        alpha: f32,
        output_loc: Point<f64, Logical>,
        scale: f32,
        aa: f32,
    ) -> Self {
        let (lx, ly) = Self::local(center_global, output_loc);
        // quad 需包络整环 + 描边宽度/2 + 抗锯齿；与圆心同尺寸方形即可。
        let reach = (radius_px + inner_w * 0.5 + aa).max(0.5) as f64;
        let size = Size::from((reach * 2., reach * 2.));
        let loc = Point::from((lx - reach, ly - reach));
        Self::build(
            size,
            loc,
            Params {
                mode: 1.0,
                color,
                center: [reach as f32, reach as f32],
                radius: radius_px,
                inner_w,
                aa,
                a0,
                a1,
                p0: [0.0, 0.0],
                p1: [0.0, 0.0],
                p2: [0.0, 0.0],
                trail_a0: 0.0,
                trail_a1: 0.0,
                alpha,
                glyph: 0.0,
                rot: 0.0,
            },
            scale,
            &[],
            None,
        )
    }

    /// BASpark `index.html:509-521` spark 三角形：白色，真实 3 顶点
    /// `(0,-s)` / `(0.6s, 0.6s)` / `(-0.6s, 0.6s)`，绕中心旋转 rot。
    pub fn spark_triangle(
        center_global: Point<f64, Logical>,
        side: f32,
        rot: f32,
        alpha: f32,
        output_loc: Point<f64, Logical>,
        scale: f32,
        aa: f32,
    ) -> Self {
        let (lx, ly) = Self::local(center_global, output_loc);
        let s = side.max(0.5) as f64;
        // AA 抗锯齿：quad 外扩 aa，三角形顶点按原半径 s 计算（顶点离 quad 边留出
        // 软边），避免三角形尖端被 quad 裁剪成硬边。
        let rc = s + aa as f64;
        let size = Size::from((rc * 2., rc * 2.));
        let loc = Point::from((lx - rc, ly - rc));

        // 三顶点（相对 quad 中心 (rc,rc)，先旋转再平移）。
        // BASpark ctx.translate(cx, cy); ctx.rotate(rot); moveTo/lineTo (0,-s)/(0.6s,0.6s)/(-0.6s,0.6s).
        let (cosr, sinr) = (rot.cos(), rot.sin());
        let rot2 = |x: f32, y: f32| -> [f32; 2] {
            [cosr * x - sinr * y, sinr * x + cosr * y]
        };
        let ctr = rc as f32;
        let sf = s as f32;
        let v0 = rot2(0.0, -sf);
        let v1 = rot2(sf * 0.6, sf * 0.6);
        let v2 = rot2(-sf * 0.6, sf * 0.6);
        let p0 = [ctr + v0[0], ctr + v0[1]];
        let p1 = [ctr + v1[0], ctr + v1[1]];
        let p2 = [ctr + v2[0], ctr + v2[1]];

        Self::build(
            size,
            loc,
            Params {
                mode: 2.0,
                // shader 内固定白色（BASpark: rgba(255,255,255,alpha)）
                color: [1.0, 1.0, 1.0],
                center: [ctr, ctr],
                radius: 0.0,
                inner_w: 0.0,
                aa,
                a0: 0.0,
                a1: 0.0,
                p0,
                p1,
                p2,
                trail_a0: 0.0,
                trail_a1: 0.0,
                alpha,
                glyph: 0.0,
                rot: 0.0,
            },
            scale,
            &[],
            None,
        )
    }

    /// 字符模式（mode 4）：随机代码字符，atlas 采样字形掩码。
    /// `center_global` 字形中心、`half_size` 字形半边长（geo px）、`glyph_index`
    /// 字形索引（0..=`glyphs::CHAR_COUNT-1`）、`rot` 旋转弧度。
    pub fn glyph(
        center_global: Point<f64, Logical>,
        half_size: f32,
        glyph_index: f32,
        rot: f32,
        color: [f32; 3],
        alpha: f32,
        output_loc: Point<f64, Logical>,
        scale: f32,
        aa: f32,
        atlas: &GlesTexture,
    ) -> Self {
        let (lx, ly) = Self::local(center_global, output_loc);
        let r = half_size.max(0.5) as f64;
        // quad 需包住字形 + 抗锯齿软边；shader 内 u_radius=字形半边长。
        let rc = r + aa as f64;
        let size = Size::from((rc * 2., rc * 2.));
        let loc = Point::from((lx - rc, ly - rc));
        Self::build(
            size,
            loc,
            Params {
                mode: 4.0,
                color,
                center: [rc as f32, rc as f32],
                radius: r as f32,
                inner_w: 0.0,
                aa,
                a0: 0.0,
                a1: 0.0,
                p0: [0.0, 0.0],
                p1: [0.0, 0.0],
                p2: [0.0, 0.0],
                trail_a0: 0.0,
                trail_a1: 0.0,
                alpha,
                glyph: glyph_index,
                rot,
            },
            scale,
            &[],
            Some(atlas),
        )
    }

    /// BASpark `index.html:360-388` `_updateTrail`：一段描边线段（lineWidth=5）
    /// + 沿投影参数 t 的线性渐变 alpha（`i/lastIdx → (i+1)/lastIdx`）。
    /// BASpark `_updateTrail`：单条 Canvas2D path 描边（`beginPath→lineTo*→stroke()`，
    /// `lineJoin=miter` 主管段连接无缝）。**根因修复**"逐段 butt-cap
    /// capsule 在连接点把 halo 归零→出现细缝如线"的断裂。
    /// `pts_global` 为完整路径点（直线模式 = trail + head；曲线模式 = 扁平化后的折线）。
    /// 全局参数 `t` 由 shader 按段索引归一化计算 → alpha 即 `t`（BASpark segGrad 拼接后是
    /// 整体 0→1 线性渐变，无需逐点 alpha 数组，避开 GL ES 2.0 数组动态索引限制）。
    pub fn trail_polyline(
        pts_global: &[Point<f64, Logical>],
        color: [f32; 3],
        output_loc: Point<f64, Logical>,
        scale: f32,
        aa: f32,
    ) -> Option<Self> {
        const MAX_TRAIL_PTS: usize = 128;
        if pts_global.len() < 2 {
            return None;
        }
        let take = pts_global.len().min(MAX_TRAIL_PTS);
        let mut local_pts: Vec<[f32; 2]> = Vec::with_capacity(take);
        let mut minx = f32::MAX;
        let mut miny = f32::MAX;
        let mut maxx = f32::MIN;
        let mut maxy = f32::MIN;
        for g in pts_global.iter().take(take) {
            let (lx, ly) = Self::local(*g, output_loc);
            let x = lx as f32;
            let y = ly as f32;
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
            local_pts.push([x, y]);
        }
        // halo 外缘半径 = 5.5（core 半径 2.5 + shadowBlur 3 外扩展）+ aa。
        let pad = 5.5 + aa;
        minx -= pad;
        miny -= pad;
        maxx += pad;
        maxy += pad;
        let w = (maxx - minx).max(1.0) as f64;
        let h = (maxy - miny).max(1.0) as f64;
        let size = Size::from((w, h));
        let loc = Point::from((minx as f64, miny as f64));
        // 平移到 quad 小坐标。
        for p in local_pts.iter_mut() {
            p[0] -= minx;
            p[1] -= miny;
        }
        Some(Self::build(
            size,
            loc,
            Params {
                mode: 3.0,
                color,
                center: [0.0, 0.0],
                radius: 0.0,
                inner_w: 0.0,
                aa,
                a0: 0.0,
                a1: 0.0,
                p0: [0.0, 0.0],
                p1: [0.0, 0.0],
                p2: [0.0, 0.0],
                trail_a0: 0.0,
                trail_a1: 0.0,
                alpha: 1.0, // niri_alpha: alpha 由 shader 内 best_t 计算，不乘 opacity
                glyph: 0.0,
                rot: 0.0,
            },
            scale,
            &local_pts,
            None,
        ))
    }

    /// GLES shader 是否就绪。
    pub fn has_shader(renderer: &mut impl NiriRenderer) -> bool {
        Shaders::get(renderer).program(ProgramType::CursorEffect).is_some()
    }
}

impl Element for CursorEffectElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for CursorEffectElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let _span = tracy_client::span!("CursorEffectElement::draw");
        frame.with_gpu_span(gpu_span_location!("CursorEffectElement::draw"), |frame| {
            // B1: BASpark 全程 `globalCompositeOperation = "lighter"`（加法混合）做发光叠加。
            // 我们的 frag 输出为预乘 alpha (`gl_FragColor = vec4(rgb*alpha, alpha)`)，
            // 故加法混合用 `BlendFunc(ONE, ONE)`（预加色与预加 alpha 项），画完还原
            // smithay 默认 `BlendFunc(ONE, ONE_MINUS_SRC_ALPHA)`（预乘 over）。
            frame.with_context(|gl| unsafe {
                gl.BlendFunc(ffi::ONE, ffi::ONE);
            })?;
            let res = RenderElement::<GlesRenderer>::draw(
                &self.inner, frame, src, dst, damage, opaque_regions, cache,
            );
            frame.with_context(|gl| unsafe {
                gl.BlendFunc(ffi::ONE, ffi::ONE_MINUS_SRC_ALPHA);
            })?;
            res
        })
    }
    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}

impl<'render> RenderElement<TtyRenderer<'render>> for CursorEffectElement {
    fn draw(
        &self,
        frame: &mut TtyFrame<'_, '_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), TtyRendererError<'render>> {
        let frame = frame.as_gles_frame();
        RenderElement::<GlesRenderer>::draw(self, frame, src, dst, damage, opaque_regions, cache)?;
        Ok(())
    }
    fn underlying_storage(
        &self,
        renderer: &mut TtyRenderer<'render>,
    ) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}

/// BASpark `index.html:420-422` `weightProp` + `getAlpha`：
/// `weight-prop(t) = min(2 - |4(t - 0.5)|, 1)`；对应两侧到中央的描边宽度峰。
#[inline]
fn weight_prop(t: f32) -> f32 {
    (2.0 - (4.0 * (t - 0.5)).abs()).min(1.0)
}

/// 收集一个输出上需要绘制的光标特效元素（1:1 翻译 BASpark `_getEffectRects` /
/// `_updateWaves` / `_updateSparks` / `_updateTrail` 的绘制顺序）。
///
/// 所有坐标都先转成输出局部坐标（screen-global 减 output_loc）。
///
/// 字符模式：当 `cursor_effect_atlas` 纹理可用时（`Shaders::compile` 时创建），
/// 全部粒子（点击爆裂/外圈/火花/拖尾）渲染为随机代码字符（a-z/A-Z/数字/符号，
/// 稳定伪随机数 [0,1)：同一 seed 恒返回同一值（字符生命周期内恒定），
/// 用于给每个字符派生独立的速度/半径/大小/淡出时间 → 不规则运动与消失。
fn hash_unit(seed: usize) -> f32 {
    let mut h = (seed as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D049BB133111EB);
    h ^= h >> 31;
    ((h & 0xFF_FFFF) as f32) / 16_777_216.0
}

/// 字符模式：全部粒子替换为代码字符（`glyphs.rs`），每 ~200ms 换一次字形
/// （`glyph_for` 相位打散，各字符在不同时刻闪烁）。
pub fn collect_render_elements(
    renderer: &mut impl NiriRenderer,
    state: &CursorEffectState,
    output_loc: Point<f64, Logical>,
    scale: f32,
    aa: f32,
) -> Vec<CursorEffectElement> {
    if !state.enabled {
        return Vec::new();
    }
    if let Some(atlas) = Shaders::get(renderer).cursor_effect_atlas.clone() {
        return collect_glyph_elements(state, output_loc, scale, aa, &atlas);
    }

    collect_geometric_elements(state, output_loc, scale, aa)
}

/// 字符模式：全部粒子替换为随机代码字符（`glyphs::FONT`，ASCII 0x21..0x7E）。
fn collect_glyph_elements(
    state: &CursorEffectState,
    output_loc: Point<f64, Logical>,
    scale: f32,
    aa: f32,
    atlas: &GlesTexture,
) -> Vec<CursorEffectElement> {
    let mut out = Vec::with_capacity(64);

    // 字符闪烁节拍：每 ~200ms 换一次字符；相位按 seed 打散（STAGGER 档），
    // 不同字符在不同时刻变化——不会全体同时闪。
    let t_ms = state
        .last_frame_time
        .duration_since(state.instant0)
        .as_secs_f64()
        * 1000.0;
    const CHANGE_MS: f64 = 200.0;
    const STAGGER: usize = 12;
    let glyph_for = |seed: usize| -> f32 {
        let phase_ms = (seed % STAGGER) as f64 * (CHANGE_MS / STAGGER as f64);
        let bucket = ((t_ms + phase_ms) / CHANGE_MS).floor() as u64;
        let h = bucket.wrapping_mul(0x85EBCA77) ^ (seed as u64).wrapping_mul(0xC2B2AE35);
        (h as usize % glyphs::CHAR_COUNT) as f32
    };

    let opacity = state.opacity;
    let fill_color = [
        state.color[0] as f32 / 255.0,
        state.color[1] as f32 / 255.0,
        state.color[2] as f32 / 255.0,
    ];

    // ─── Waves: filledCircle + rings → 字符（BASpark `_updateWaves`） ───
    for (wi, w) in state.waves.iter().enumerate() {
        // 运动：半径/环位置仍由 MAX_LIFE 推进（速度不变）。
        let wave_prog = (w.life / state::filled_cfg::MAX_LIFE).clamp(0.0, 1.0);
        let ring_prog = (w.life / state::rings_cfg::MAX_LIFE).clamp(0.0, 1.0);
        // 淡出：用 FADE_EXTEND 延长窗口 → 消失更慢。
        let wave_fade =
            (w.life / (state::filled_cfg::MAX_LIFE * state::filled_cfg::FADE_EXTEND)).clamp(0.0, 1.0);
        let ring_fade =
            (w.life / (state::rings_cfg::MAX_LIFE * state::rings_cfg::FADE_EXTEND)).clamp(0.0, 1.0);

        let ease = 1.0 - (1.0 - wave_prog).powi(3);
        let fill_r = state::filled_cfg::R_ADD_RATE * state.scale * ease;
        let fill_alpha = (1.0 - wave_fade).max(0.0) * opacity;
        if fill_alpha > 0.0 {
            // 蓝色爆裂圆盘：点阵铺满圆（BASpark filledCircle 是整块圆盘）。
            let r = fill_r.max(5.0);
            // 圆盘字符只铺到 0.85r：圆环（在 r 处）落在盘外，不压盘边字符。
            let disk_r = r * 0.85;
            let spacing = (disk_r / 3.5).max(4.0);
            let cell = (disk_r / spacing).ceil() as i32;
            let mut cell_i = 0usize;
            for gy in -cell..=cell {
                for gx in -cell..=cell {
                    let dx = gx as f32 * spacing;
                    let dy = gy as f32 * spacing;
                    if dx * dx + dy * dy > disk_r * disk_r {
                        continue;
                    }
                    cell_i += 1;
                    let seed_c = (w.seed as usize) ^ (70000 + wi * 4096 + cell_i);
                    // 每格独立不规则：位置抖动、大小、亮度、淡出时间全部派生自 seed。
                    let jx = (hash_unit(seed_c) - 0.5) * spacing * 0.6;
                    let jy = (hash_unit(seed_c ^ 0x3C1) - 0.5) * spacing * 0.6;
                    // 更大更实：尺寸 1.2..1.5×间距（重叠多）、亮度 0.85..1.0。
                    let sz = (spacing * (1.2 + hash_unit(seed_c ^ 0x5A5) * 0.3)).max(5.0);
                    let brightness = 0.85 + hash_unit(seed_c ^ 0x1E1) * 0.15; // 0.85..1.0
                    // 淡出时间不规则且缓慢：起点 0..0.4、终点 0.5..1.0 散开
                    // （最长差 ~32 帧）→ 各格明显在不同时刻消失。
                    let fade_start_c = hash_unit(seed_c ^ 0x9E3) * 0.4; // 0..0.4
                    let fade_end_c = 0.5 + hash_unit(seed_c ^ 0xF1) * 0.5; // 0.5..1.0
                    let cell_fade =
                        ((wave_fade - fade_start_c) / (fade_end_c - fade_start_c)).clamp(0.0, 1.0);
                    let alpha_c = (1.0 - cell_fade) * fill_alpha * brightness;
                    if alpha_c <= 0.0 {
                        continue;
                    }
                    out.push(CursorEffectElement::glyph(
                        Point::from((w.x as f64 + dx as f64 + jx as f64, w.y as f64 + dy as f64 + jy as f64)),
                        sz,
                        glyph_for(seed_c),
                        0.0,
                        fill_color,
                        alpha_c,
                        output_loc,
                        scale,
                        aa,
                        atlas,
                    ));
                }
            }
        }

        let ring_alpha = state.ring_alpha(ring_fade);
        let ring_rgb = state.ring_rgb_at(ring_prog);
        if ring_alpha <= 0.0 {
            continue;
        }
        for seg in &w.ring.segs {
            let seg_off = seg.off;
            // 环淡出不"向固定点收缩"，字符始终分布在整段弧上；淡出改为
            // 每个字符独立不规则的透明度渐变 → 缓慢透明消失、不一起没。
            // （保留 LEN_STOP_ADD 阶段的"环从弧头生长出来"）
            let len = if ring_fade <= state::rings_cfg::LEN_STOP_ADD_POINT {
                let frac = if state::rings_cfg::LEN_STOP_ADD_POINT > 0.0 {
                    ring_fade / state::rings_cfg::LEN_STOP_ADD_POINT
                } else {
                    1.0
                };
                seg.len * frac
            } else {
                seg.len
            };

            let radius = w.r + seg.r_round_rate * state.scale; // index.html:477
            let seg_num = state::rings_cfg::SEG_NUM;
            // 字符版：子段隔一个取一个 → 环上字符更少、更大。
            for k in (0..seg_num).step_by(2) {
                let t0 = k as f32 / seg_num as f32;
                let t1 = (k + 1) as f32 / seg_num as f32;
                let seed_k = (w.seed as usize) ^ (1000 + wi * 64 + k);
                // 每字符独立角速度系数 0.6..1.4（继承环旋转）。
                let speed_k = 0.6 + hash_unit(seed_k ^ 0x11) * 0.8;
                // 每字符独立自转速度 0.04..0.12 rad/帧：有下限、各不相同（像落叶），
                // 保证即使环本身不转（rs=0）每个字符也在动。
                let spin_k = 0.04 + hash_unit(seed_k ^ 0x55) * 0.08;
                // 每字符独立半径 0.85..1.15 → 环不再是一个正圆，逐个漂移。
                let rad_k = 0.85 + hash_unit(seed_k ^ 0x22) * 0.3;
                // 每字符独立淡出窗口：起点 0.1..0.5、时长 ≥0.3 窗口（≥28 帧）、
                // 终点散布到 0.4..1.0（最长差 ~55 帧）→ 明显逐个消失而非一起没。
                let fade_start_k = 0.1 + hash_unit(seed_k ^ 0x33) * 0.4; // 0.1..0.5
                let fade_end_k =
                    (fade_start_k + 0.3 + hash_unit(seed_k ^ 0x44) * 0.4).min(1.0); // 0.4..1.0
                let fade_prog =
                    ((ring_fade - fade_start_k) / (fade_end_k - fade_start_k)).clamp(0.0, 1.0);
                let a0 = seg_off + len * t0;
                let a1 = seg_off + len * t1;
                if (a1 - a0).abs() < 0.01 {
                    continue;
                }
                let mid_arc = (a0 + a1) * 0.5;
                // 位置 = 弧内相对角 + 继承环旋转 + 每字符独立自转（随时间累加）。
                let g_angle = mid_arc + w.ring.ang * speed_k + w.life * spin_k;
                let g_radius = radius * rad_k;

                let w_t = weight_prop(t0);
                let lw = (state::rings_cfg::MIN_W * (1.0 - w_t)
                    + state::rings_cfg::MAX_W * w_t)
                    * (-0.8 * (ring_fade - 0.8) + 1.0).min(1.0);
                if lw <= 0.0 {
                    continue;
                }
                // 每字符独立淡出窗口 → 消失时间不规则，缓慢渐隐。
                let char_alpha = (1.0 - fade_prog) * ring_alpha;
                if char_alpha <= 0.0 {
                    continue;
                }
                let gx = w.x + g_radius * g_angle.cos();
                let gy = w.y + g_radius * g_angle.sin();
                out.push(CursorEffectElement::glyph(
                    Point::from((gx as f64, gy as f64)),
                    (lw * 3.0).max(5.5),
                    glyph_for(seed_k),
                    0.0,
                    ring_rgb,
                    char_alpha,
                    output_loc,
                    scale,
                    aa,
                    atlas,
                ));
            }
        }
    }

    // ─── Sparks → 字符（BASpark `_updateSparks`，顶点旋转） ───
    for (si, s) in state.sparks.iter().enumerate() {
        let alpha = s.a.max(0.0) * opacity;
        if alpha <= 0.0 {
            continue;
        }
        out.push(CursorEffectElement::glyph(
            Point::from((s.x as f64, s.y as f64)),
            (s.s * 1.3).max(6.0),
            glyph_for(5000 + si),
            s.rot,
            [1.0, 1.0, 1.0],
            alpha,
            output_loc,
            scale,
            aa,
            atlas,
        ));
    }

    // ─── Trail → 字符链（BASpark `_updateTrail` 的路径点，逐点字符） ───
    // 拖尾点间距 ~2px；字符放大到 13px、每 ~10px 一个 → 少而大、代码流风格。
    let head = if state.is_down || state.persistent_trail {
        state.last_pos
    } else {
        None
    };
    let n = state.trail.len();
    for (i, p) in state.trail.iter().enumerate().step_by(5) {
        if p.life <= 0.0 {
            continue;
        }
        let t = if n > 1 { i as f32 / (n as f32 - 1.0) } else { 1.0 };
        // BASpark segGrad 线性渐变：尾部淡、头部亮。
        let alpha = (0.08 + t * 0.87).min(0.95);
        out.push(CursorEffectElement::glyph(
            Point::from((p.x as f64, p.y as f64)),
            9.0,
            glyph_for(p.id as usize),
            0.0,
            fill_color,
            alpha,
            output_loc,
            scale,
            aa,
            atlas,
        ));
    }
    if let Some((hx, hy)) = head {
        out.push(CursorEffectElement::glyph(
            Point::from((hx as f64, hy as f64)),
            11.0,
            glyph_for(12345),
            0.0,
            fill_color,
            1.0,
            output_loc,
            scale,
            aa,
            atlas,
        ));
    }

    out
}

/// 几何模式（atlas 不可用时的回退）：SDF 复刻 BASpark 矢量粒子。
fn collect_geometric_elements(
    state: &CursorEffectState,
    output_loc: Point<f64, Logical>,
    scale: f32,
    aa: f32,
) -> Vec<CursorEffectElement> {
    if !state.enabled {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(
        state.waves.len() * (1 + 2 * state::rings_cfg::SEG_NUM)
            + state.sparks.len()
            + state.trail.len(),
    );

    let opacity = state.opacity;
    let color_u8 = state.color;
    let fill_color = [
        color_u8[0] as f32 / 255.0,
        color_u8[1] as f32 / 255.0,
        color_u8[2] as f32 / 255.0,
    ];

    // ─── Waves: filledCircle + 外圈分段弧 ─ BASpark `_updateWaves` ───
    for w in &state.waves {
        // 运动：半径/环位置仍由 MAX_LIFE 推进；淡出用 FADE_EXTEND 延长窗口。
        let wave_prog = (w.life / state::filled_cfg::MAX_LIFE).clamp(0.0, 1.0);
        let ring_prog = (w.life / state::rings_cfg::MAX_LIFE).clamp(0.0, 1.0);
        let wave_fade =
            (w.life / (state::filled_cfg::MAX_LIFE * state::filled_cfg::FADE_EXTEND)).clamp(0.0, 1.0);
        let ring_fade =
            (w.life / (state::rings_cfg::MAX_LIFE * state::rings_cfg::FADE_EXTEND)).clamp(0.0, 1.0);

        // filledCircle：rAddRate * scale * cubic-out ease
        let ease = 1.0 - (1.0 - wave_prog).powi(3);
        let fill_r = state::filled_cfg::R_ADD_RATE * state.scale * ease;
        let fill_alpha = (1.0 - wave_fade).max(0.0) * opacity;
        if fill_alpha > 0.0 {
            out.push(CursorEffectElement::filled_circle(
                Point::from((w.x as f64, w.y as f64)),
                fill_r,
                fill_color,
                fill_alpha,
                output_loc,
                scale,
                aa,
            ));
        }

        // rings：每 wave 有 ring.ang、ring.rs、ring.segs[2]，每段再分 SEG_NUM 子段
        // (index.html:419-482). 描边色用 `ring_rgb_at(ring_prog)` 渐变，alpha 用 `ring_alpha`.
        // B2: BASpark ring `strokeStyle = rgba(rr,gg,bb,alphaRing)` 不走 `this.alpha()`
        //（index.html:469-469），即不乘 opacity —— 只有 filledCircle/sparks 乘 opacity。
        let ring_alpha = state.ring_alpha(ring_fade);
        let ring_rgb = state.ring_rgb_at(ring_prog);
        let line_width_mul = (-0.8 * (ring_fade - 0.8) + 1.0).min(1.0); // index.html:462
        if ring_alpha <= 0.0 || line_width_mul <= 0.0 {
            continue;
        }
        let center = Point::from((w.x as f64, w.y as f64));
        for seg in &w.ring.segs {
            // index.html:426-455 计算每段 [start, end] 角度区间。
            let base = w.ring.ang + seg.off;
            let (start, end) = if ring_fade <= state::rings_cfg::LEN_STOP_ADD_POINT {
                let frac = if state::rings_cfg::LEN_STOP_ADD_POINT > 0.0 {
                    ring_fade / state::rings_cfg::LEN_STOP_ADD_POINT
                } else {
                    1.0
                };
                let len = seg.len * frac;
                let end = base + seg.len;
                (end - len, end)
            } else if ring_fade > state::rings_cfg::LEN_START_DIM_POINT {
                let frac = (ring_fade - state::rings_cfg::LEN_START_DIM_POINT)
                    / (1.0 - state::rings_cfg::LEN_START_DIM_POINT);
                let len = seg.len * (1.0 - frac).max(0.0);
                let start = base;
                (start, start + len)
            } else {
                (base, base + seg.len)
            };

            let radius = w.r + seg.r_round_rate * state.scale; // index.html:477

            // SEG_NUM 子段，每子段 lineWidth 用 weightProp(pts[0]).
            let seg_num = state::rings_cfg::SEG_NUM;
            for k in 0..seg_num {
                let t0 = k as f32 / seg_num as f32;
                let t1 = (k + 1) as f32 / seg_num as f32;
                let a0 = start + (end - start) * t0;
                let a1 = start + (end - start) * t1;
                if (a1 - a0).abs() < 0.01 {
                    continue; // index.html:459
                }
                let w_t = weight_prop(t0); // index.html:469
                let lw = (state::rings_cfg::MIN_W * (1.0 - w_t)
                    + state::rings_cfg::MAX_W * w_t)
                    * line_width_mul;
                if lw <= 0.0 {
                    continue;
                }
                // B7: BASpark `ctx.arc(wx,wy,radius,a0,a1)` 原样接受任意 a0<a1（可跨越
                // ±π 边界）。不再在 Rust 侧 wrap+丢弃，把原始角度直接传给 shader，
                // shader 用 [0,2π) 归一化 + 前向弧长判定，正确绘制跨边界圆弧。
                out.push(CursorEffectElement::ring_segment(
                    center,
                    radius,
                    lw,
                    a0,
                    a1,
                    ring_rgb,
                    ring_alpha,
                    output_loc,
                    scale,
                    aa,
                ));
            }
        }
    }

    // ─── Sparks: 白色三角形（BASpark `_updateSparks`，顶点旋转） ─ index.html:509-521
    for s in &state.sparks {
        let alpha = s.a.max(0.0) * opacity;
        if alpha <= 0.0 {
            continue;
        }
        out.push(CursorEffectElement::spark_triangle(
            Point::from((s.x as f64, s.y as f64)),
            s.s,
            s.rot,
            alpha,
            output_loc,
            scale,
            aa,
        ));
    }

    // ─── Trail: 单条连续 path 描边（BASpark `_updateTrail` lineWidth=5，根因修复
    // 段连接处断裂：单 quad 包络整条 trail，shader 内整条折线的最小 distance field 一次画完，
    // 消除逐段 butt-cap 在连接点把 halo 归零的细缝"线一样断裂"）。
    // head（光标当前点）只在拖尾**正在被喂入**时才接到折线末端。否则（松开按键后
    // 静止的旧 trail 点 + 移开的光标）会把旧 trail 与光标用直线连起来 → 点击 A 后
    // 移到 B 就画出笔直的 A→B 线段（"锁定鼠标成线"）；且移动期间 trail 不更新，
    // 中间晃荡完全不可见。松开后 trail 仅随衰减消失，不再连向光标。
    let head = if state.is_down || state.persistent_trail {
        state.last_pos
    } else {
        None
    };
    let mut pts: Vec<(f32, f32)> = state.trail.iter().map(|p| (p.x, p.y)).collect();
    if let Some((hx, hy)) = head {
        pts.push((hx, hy));
    }
    if pts.len() >= 2 {
        let polyline_pts: Vec<Point<f64, Logical>> = if state.apply_curve_draw {
            // 曲线版：Catmull-Rom → Cubic Bezier 扁平化（BASpark `ApplyCurveDraw` 拼接，
            // index.html:341-373）。每段三次贝塞尔扁平为 CURVE_SUBSEGS 个折线点，
            // 喂给同一 polyline SDF shader（GPU 曲线光栅化本质即折线化；5px 描边
            // + 张力 /6 的曲率下肉眼不辨，折线 SDF 在拐角处自然 min() 圆滑过渡）。
            const CURVE_SUBSEGS: usize = 8;
            let last_idx = pts.len() - 1;
            let mut out_pts: Vec<Point<f64, Logical>> = Vec::new();
            for i in 0..last_idx {
                let a = pts[i];
                let b = pts[i + 1];
                if i == 0 {
                    out_pts.push(Point::from((a.0 as f64, a.1 as f64)));
                }
                // BASpark `index.html:347-353` Catmull-Rom 控制点 (张力系数 /6)。
                let prev = if i > 0 { pts[i - 1] } else { a };
                let next = if i < last_idx - 1 { pts[i + 2] } else { b };
                let cp1x = a.0 + (b.0 - prev.0) / 6.0;
                let cp1y = a.1 + (b.1 - prev.1) / 6.0;
                let cp2x = b.0 - (next.0 - a.0) / 6.0;
                let cp2y = b.1 - (next.1 - a.1) / 6.0;
                let n = CURVE_SUBSEGS as f32;
                for k in 1..=CURVE_SUBSEGS {
                    let t = k as f32 / n;
                    let mt = 1.0 - t;
                    // de Casteljau: B(t) = (1-t)^3 a + 3(1-t)^2 t cp1 + 3(1-t) t^2 cp2 + t^3 b
                    let bx = mt * mt * mt * a.0
                        + 3.0 * mt * mt * t * cp1x
                        + 3.0 * mt * t * t * cp2x
                        + t * t * t * b.0;
                    let by = mt * mt * mt * a.1
                        + 3.0 * mt * mt * t * cp1y
                        + 3.0 * mt * t * t * cp2y
                        + t * t * t * b.1;
                    out_pts.push(Point::from((bx as f64, by as f64)));
                }
            }
            out_pts
        } else {
            // 直线版（BASpark `_updateTrail` index.html:375-388）。直接用原始点序列。
            pts.iter()
                .map(|p| Point::from((p.0 as f64, p.1 as f64)))
                .collect()
        };
        // BASpark `_updateTrail`：5px lineWidth 描边 + shadowBlur=3 halo，不乘 opacity。
        // alpha 走全局位置线陆渐变 (segGrad 缝合 0→1，shader 内由段索引归一化 best_t 求值)。
        if let Some(el) =
            CursorEffectElement::trail_polyline(&polyline_pts, fill_color, output_loc, scale, aa)
        {
            out.push(el);
        }
    } else if state.trail.len() == 1 {
        // BASpark `index.html:331-337` 单点小圆：`fade = max(0, life)`，半径 `2.5+2*fade`，
        // fillStyle `rgba(color, fade*0.85)` —— 同样不走 this.alpha()，不乘 opacity。
        let p = state.trail.front().unwrap();
        let fade = p.life.max(0.0);
        if fade > 0.0 {
            out.push(CursorEffectElement::filled_circle(
                Point::from((p.x as f64, p.y as f64)),
                2.5 + 2.0 * fade,
                fill_color,
                fade * 0.85,
                output_loc,
                scale,
                aa,
            ));
        }
    }

    out
}
