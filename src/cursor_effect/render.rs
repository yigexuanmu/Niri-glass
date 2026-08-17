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
use smithay::backend::renderer::gles::{ffi, GlesError, GlesFrame, GlesRenderer, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::gpu_span_location;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
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
    ) -> Self {
        let area = Vec2::new(size.w as f32, size.h as f32);
        // 几何充满整个 quad，`geo_loc=0`，所以 `input_to_geo = Scale(area)`，
        // `coords_geo = niri_v_coords * area`。
        let input_to_geo = Mat3::from_scale(area);
        let geo_size = area;

        let mut inner = ShaderRenderElement::empty(ProgramType::CursorEffect, Kind::Unspecified);
        inner.update(
            size,
            None,
            scale,
            params.alpha,
            Rc::new([
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
            ]),
            HashMap::new(),
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
        let size = Size::from((r * 2., r * 2.));
        let loc = Point::from((lx - r, ly - r));
        Self::build(
            size,
            loc,
            Params {
                mode: 0.0,
                color,
                center: [r as f32, r as f32],
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
            },
            scale,
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
            },
            scale,
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
        let size = Size::from((s * 2., s * 2.));
        let loc = Point::from((lx - s, ly - s));

        // 三顶点（相对 quad 中心 (s,s)，先旋转再平移）。
        // BASpark ctx.translate(cx, cy); ctx.rotate(rot); moveTo/lineTo (0,-s)/(0.6s,0.6s)/(-0.6s,0.6s).
        let (cosr, sinr) = (rot.cos(), rot.sin());
        let rot2 = |x: f32, y: f32| -> [f32; 2] {
            [cosr * x - sinr * y, sinr * x + cosr * y]
        };
        let ctr = s as f32;
        let v0 = rot2(0.0, -ctr);
        let v1 = rot2(ctr * 0.6, ctr * 0.6);
        let v2 = rot2(-ctr * 0.6, ctr * 0.6);
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
            },
            scale,
        )
    }

    /// BASpark `index.html:360-388` `_updateTrail`：一段描边线段（lineWidth=5）
    /// + 沿投影参数 t 的线性渐变 alpha（`i/lastIdx → (i+1)/lastIdx`）。
    pub fn trail_segment(
        a_global: Point<f64, Logical>,
        b_global: Point<f64, Logical>,
        line_width: f32,
        alpha_start: f32,
        alpha_end: f32,
        color: [f32; 3],
        alpha: f32,
        output_loc: Point<f64, Logical>,
        scale: f32,
        aa: f32,
    ) -> Self {
        let (lax, lay) = Self::local(a_global, output_loc);
        let (lbx, lby) = Self::local(b_global, output_loc);
        let pad = (line_width * 0.5 + aa) as f64;
        let minx = lax.min(lbx) - pad;
        let miny = lay.min(lby) - pad;
        let maxx = lax.max(lbx) + pad;
        let maxy = lay.max(lby) + pad;
        let w = (maxx - minx).max(1.0);
        let h = (maxy - miny).max(1.0);
        let size = Size::from((w, h));
        let loc = Point::from((minx, miny));
        // quad 的 p0 = (a 在 quad 内), p1 = (b 在 quad 内)
        let p0 = [(lax - minx) as f32, (lay - miny) as f32];
        let p1 = [(lbx - minx) as f32, (lby - miny) as f32];
        Self::build(
            size,
            loc,
            Params {
                mode: 3.0,
                color,
                center: [0.0, 0.0],
                radius: 0.0,
                inner_w: line_width,
                aa,
                a0: 0.0,
                a1: 0.0,
                p0,
                p1,
                p2: [0.0, 0.0],
                trail_a0: alpha_start,
                trail_a1: alpha_end,
                alpha,
            },
            scale,
        )
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
pub fn collect_render_elements(
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
        let wave_prog = (w.life / state::filled_cfg::MAX_LIFE).clamp(0.0, 1.0);
        let ring_prog = (w.life / state::rings_cfg::MAX_LIFE).clamp(0.0, 1.0);

        // filledCircle：rAddRate * scale * cubic-out ease
        let ease = 1.0 - (1.0 - wave_prog).powi(3);
        let fill_r = state::filled_cfg::R_ADD_RATE * state.scale * ease;
        let fill_alpha = (1.0 - wave_prog).max(0.0) * opacity;
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
        let ring_alpha = state.ring_alpha(ring_prog);
        let ring_rgb = state.ring_rgb_at(ring_prog);
        let line_width_mul = (-0.8 * (ring_prog - 0.8) + 1.0).min(1.0); // index.html:462
        if ring_alpha <= 0.0 || line_width_mul <= 0.0 {
            continue;
        }
        let center = Point::from((w.x as f64, w.y as f64));
        for seg in &w.ring.segs {
            // index.html:426-455 计算每段 [start, end] 角度区间。
            let base = w.ring.ang + seg.off;
            let (start, end) = if ring_prog <= state::rings_cfg::LEN_STOP_ADD_POINT {
                let frac = if state::rings_cfg::LEN_STOP_ADD_POINT > 0.0 {
                    ring_prog / state::rings_cfg::LEN_STOP_ADD_POINT
                } else {
                    1.0
                };
                let len = seg.len * frac;
                let end = base + seg.len;
                (end - len, end)
            } else if ring_prog > state::rings_cfg::LEN_START_DIM_POINT {
                let frac = (ring_prog - state::rings_cfg::LEN_START_DIM_POINT)
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

    // ─── Trail: 段渐变 alpha（BASpark `_updateTrail` lineWidth=5） ─ index.html:360-388
    // pts = trail.concat([head])（BASpark：head = lastPos）。
    let head = state.last_pos;
    let pts_count = state.trail.len() + if head.is_some() { 1 } else { 0 };
    if pts_count >= 2 {
        // 复制 trail，结尾追加 head 形成完整路径点序列。
        let mut pts: Vec<(f32, f32, f32)> =
            state.trail.iter().map(|p| (p.x, p.y, p.life)).collect();
        if let Some((hx, hy)) = head {
            // head 视为 life=1（BASpark 追加 `{ x: head.x, y: head.y, life: 1 }`）。
            pts.push((hx, hy, 1.0));
        }
        let last_idx = pts.len() - 1;
        for i in 0..last_idx {
            let a = pts[i];
            let b = pts[i + 1];
            // B3: BASpark `segGrad.addColorStop(k, rgba(color, k/lastIdx))`（index.html:356-360）
            // 纯位置渐变，不乘 life、不乘 opacity（不走 this.alpha()）。life 只在
            // `update_trail` 里决定该点是否回收，不参与绘制 alpha。
            let sa = i as f32 / last_idx as f32;
            let sb = (i + 1) as f32 / last_idx as f32;
            if sa <= 0.0 && sb <= 0.0 {
                continue;
            }
            // BASpark `_updateTrail`（index.html:334-336）给 5px trail 描边加 `ctx.shadowBlur=3`
            // 外光晕 (shadowColor=rgba(color,0.6))，halo 与 core 都跟随段 alpha 渐变
            // (segGrad, 不乘 opacity)。单一 shader 同时画 core(5px 硬) + halo(3px gaussian
            // falloff) + butt caps，替代旧的实心 11px halo + 独立 core (过度不透明 +
            // junction 高亮)。quad 须包络到 halo 外缘 r=5.5，diameter=11。
            out.push(CursorEffectElement::trail_segment(
                Point::from((a.0 as f64, a.1 as f64)),
                Point::from((b.0 as f64, b.1 as f64)),
                11.0, // halo outer diameter (core 5 + 2*shadowBlur 3 = 11)
                sa,
                sb,
                fill_color,
                1.0, // niri_alpha — shader computes combined cov internally
                output_loc,
                scale,
                aa,
            ));
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
