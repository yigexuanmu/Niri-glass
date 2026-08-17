//! 光标特效 GLES 渲染层（Cursor Effects Rendering）。
//!
//! 以 `BorderRenderElement` 为模板，包装 `ShaderRenderElement` + `ProgramType::CursorEffect`，
//! 把 `CursorEffectState` 里的粒子几何用矢量 + SDF 画出来。每基元一个 element，
//! 顶点 shader（`texture.vert`）已提供 `niri_v_coords`/`niri_size` 等自动 uniform。

use std::collections::HashMap;
use std::rc::Rc;

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, Uniform};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::gpu_span_location;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform};

use crate::backend::tty::{TtyFrame, TtyRenderer, TtyRendererError};
use crate::render_helpers::renderer::{AsGlesFrame as _, NiriRenderer};
use crate::render_helpers::shader_element::ShaderRenderElement;
use crate::render_helpers::shaders::{ProgramType, Shaders};

use crate::cursor_effect::state::CursorEffectState;

#[derive(Debug, Clone, Copy)]
struct Params {
    mode: f32,
    color: [f32; 3],
    center: [f32; 2],
    radius: f32,
    inner_w: f32,
    aa: f32,
    trail_a0: f32,
    trail_a1: f32,
    alpha: f32,
}

/// 光标特效里一个粒子基元的渲染元素。
#[derive(Debug, Clone)]
pub struct CursorEffectElement {
    inner: ShaderRenderElement,
}

impl CursorEffectElement {
    fn build(
        size: Size<f64, Logical>,
        loc: Point<f64, Logical>,
        params: Params,
        scale: f32,
    ) -> Self {
        let mut inner = ShaderRenderElement::empty(ProgramType::CursorEffect, Kind::Unspecified);
        inner.update(
            size,
            None,
            scale,
            params.alpha,
            Rc::new([
                Uniform::new("u_mode", params.mode),
                Uniform::new("u_color", params.color),
                Uniform::new("u_center", params.center),
                Uniform::new("u_radius", params.radius),
                Uniform::new("u_inner_w", params.inner_w),
                Uniform::new("u_aa", params.aa),
                Uniform::new("u_trail_a0", params.trail_a0),
                Uniform::new("u_trail_a1", params.trail_a1),
            ]),
            HashMap::new(),
        );
        inner = inner.with_location(loc);
        Self { inner }
    }

    fn local(center_global: Point<f64, Logical>, output_loc: Point<f64, Logical>) -> (f64, f64) {
        (center_global.x - output_loc.x, center_global.y - output_loc.y)
    }

    /// BASpark filledCircle：实心圆。
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
        let r = (radius_px.max(0.5)) as f64;
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
                trail_a0: 0.0,
                trail_a1: 0.0,
                alpha,
            },
            scale,
        )
    }

    /// BASpark ring（整环近似；T6 再做弧段切割）。
    pub fn ring(
        center_global: Point<f64, Logical>,
        radius_px: f32,
        inner_w: f32,
        color: [f32; 3],
        alpha: f32,
        output_loc: Point<f64, Logical>,
        scale: f32,
        aa: f32,
    ) -> Self {
        let (lx, ly) = Self::local(center_global, output_loc);
        let r = (radius_px.max(0.5)) as f64;
        let reach = r + (inner_w * 0.5 + aa) as f64;
        let size = Size::from((reach * 2., reach * 2.));
        let loc = Point::from((lx - reach, ly - reach));
        Self::build(
            size,
            loc,
            Params {
                mode: 1.0,
                color,
                center: [reach as f32, reach as f32],
                radius: r as f32,
                inner_w,
                aa,
                trail_a0: 0.0,
                trail_a1: 0.0,
                alpha,
            },
            scale,
        )
    }

    /// BASpark triangle 碎片（T4 简化：用 quad 包络，shader mode=2 全 alpha）。
    pub fn triangle(
        center_global: Point<f64, Logical>,
        side_px: f32,
        _rot: f32,
        color: [f32; 3],
        alpha: f32,
        output_loc: Point<f64, Logical>,
        scale: f32,
    ) -> Self {
        let (lx, ly) = Self::local(center_global, output_loc);
        let s = (side_px.max(0.5)) as f64;
        let size = Size::from((s * 2., s * 2.));
        let loc = Point::from((lx - s, ly - s));
        Self::build(
            size,
            loc,
            Params {
                mode: 2.0,
                color,
                center: [s as f32, s as f32],
                radius: 0.0,
                inner_w: 0.0,
                aa: 1.5,
                trail_a0: 0.0,
                trail_a1: 0.0,
                alpha,
            },
            scale,
        )
    }

    /// BASpark trail band：两 trail 点之间的带宽片段。
    pub fn trail_segment(
        a_global: Point<f64, Logical>,
        b_global: Point<f64, Logical>,
        band_w: f32,
        a0: f32,
        a1: f32,
        color: [f32; 3],
        alpha: f32,
        output_loc: Point<f64, Logical>,
        scale: f32,
    ) -> Self {
        let (lax, lay) = Self::local(a_global, output_loc);
        let (lbx, lby) = Self::local(b_global, output_loc);
        let minx = lax.min(lbx);
        let miny = lay.min(lby);
        let maxx = lax.max(lbx);
        let maxy = lay.max(lby);
        let w = (maxx - minx) + band_w as f64;
        let h = (maxy - miny) + band_w as f64;
        let size = Size::from((w, h));
        let loc = Point::from((minx - band_w as f64 * 0.5, miny - band_w as f64 * 0.5));
        Self::build(
            size,
            loc,
            Params {
                mode: 3.0,
                color,
                center: [(w * 0.5) as f32, (h * 0.5) as f32],
                radius: 0.0,
                inner_w: band_w,
                aa: 1.5,
                trail_a0: a0,
                trail_a1: a1,
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
            RenderElement::<GlesRenderer>::draw(
                &self.inner, frame, src, dst, damage, opaque_regions, cache,
            )
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

/// 收集一个输出上需要绘制的光标特效元素。
///
/// 把 `CursorEffectState` 中 waves/sparks/trail 的几何翻译成 `CursorEffectElement`。
/// 坐标全部是输出局部坐标（screen-global 减 output_loc）。
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
        state.waves.len() * 3 + state.sparks.len() + state.trail.len().saturating_sub(1),
    );

    let color_u8 = state.color;
    let color_f = [color_u8[0] as f32 / 255.0, color_u8[1] as f32 / 255.0, color_u8[2] as f32 / 255.0];
    let opacity = state.opacity;

    // Waves: filledCircle + 外环 ring。
    for w in &state.waves {
        let wave_prog = (w.life / crate::cursor_effect::state::filled_cfg::MAX_LIFE).clamp(0.0, 1.0);
        let ease = 1.0 - (1.0 - wave_prog).powi(3); // BASpark cubic out
        let r = 26.0 * state.scale * ease; // FILLED_CIRCLE_CFG.rAddRate * scale * ease
        let alpha = (1.0 - wave_prog).max(0.0) * opacity;
        if alpha <= 0.0 {
            continue;
        }
        let center = Point::from((w.x as f64, w.y as f64));
        out.push(CursorEffectElement::filled_circle(
            center, r, color_f, alpha, output_loc, scale, aa,
        ));

        // ring：半径走 R_ROUND_RATE 偏移 + ring lifetime 衰减。
        let ring_prog = (w.life / crate::cursor_effect::state::rings_cfg::MAX_LIFE).clamp(0.0, 1.0);
        let ring_alpha = state.ring_alpha(ring_prog) * opacity;
        if ring_alpha > 0.0 {
            let ring_r = r * 2.0; // T4 简化：外环半径 ≈ filled × 2
            let ring_color = state.ring_rgb_at(ring_prog);
            out.push(CursorEffectElement::ring(
                center, ring_r, 2.5, ring_color, ring_alpha, output_loc, scale, aa,
            ));
        }
    }

    // Sparks: 三角形碎片。
    for s in &state.sparks {
        let alpha = s.a.max(0.0) * opacity; // BASpark Fragment.a 是衰减 alpha
        if alpha <= 0.0 {
            continue;
        }
        let center = Point::from((s.x as f64, s.y as f64));
        let side = s.s; // s == BASpark 碎片尺寸
        out.push(CursorEffectElement::triangle(
            center, side, s.rot, color_f, alpha, output_loc, scale,
        ));
    }

    // Trail: 相邻点之间画 band。
    if state.trail.len() >= 2 {
        let pts: Vec<_> = state.trail.iter().collect();
        for pair in pts.windows(2) {
            let a = pair[0];
            let b = pair[1];
            let a0 = a.life * opacity;
            let a1 = b.life * opacity;
            if a0 <= 0.0 && a1 <= 0.0 {
                continue;
            }
            let pa = Point::from((a.x as f64, a.y as f64));
            let pb = Point::from((b.x as f64, b.y as f64));
            out.push(CursorEffectElement::trail_segment(
                pa, pb, 3.0, a0, a1, color_f, 1.0, output_loc, scale,
            ));
        }
    } else if state.trail.len() == 1 {
        // 单点拖尾：画个小圆代表。
        let p = state.trail.front().unwrap();
        let alpha = p.life * opacity;
        if alpha > 0.0 {
            let center = Point::from((p.x as f64, p.y as f64));
            out.push(CursorEffectElement::filled_circle(
                center, 3.0, color_f, alpha, output_loc, scale, aa,
            ));
        }
    }

    out
}
