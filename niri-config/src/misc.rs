use crate::appearance::{Color, WorkspaceShadow, WorkspaceShadowPart, DEFAULT_BACKDROP_COLOR};
use crate::utils::{parse_arg_node, Flag, MergeWith};
use crate::FloatOrInt;
use std::str::FromStr;

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct SpawnAtStartup {
    #[knuffel(arguments)]
    pub command: Vec<String>,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct SpawnShAtStartup {
    #[knuffel(argument)]
    pub command: String,
}

#[derive(Debug, PartialEq)]
pub struct Cursor {
    pub xcursor_theme: String,
    pub xcursor_size: u8,
    pub hide_when_typing: bool,
    pub hide_after_inactive_ms: Option<u32>,
    pub shake_to_enlarge: Option<ShakeToEnlarge>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShakeToEnlarge {
    pub off: bool,
    pub zoom_factor: f64,
    pub hold_duration_ms: u32,
    pub threshold: f64,
    pub grow: bool,
    pub grow_speed: f64,
}

impl Default for ShakeToEnlarge {
    fn default() -> Self {
        Self {
            off: false,
            zoom_factor: 5.0,
            hold_duration_ms: 1500,
            threshold: 2000.0,
            grow: false,
            grow_speed: 0.01,
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct ShakeToEnlargePart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child, unwrap(argument))]
    pub zoom_factor: Option<FloatOrInt<0, { i32::MAX }>>,
    #[knuffel(child, unwrap(argument))]
    pub hold_duration_ms: Option<u32>,
    #[knuffel(child, unwrap(argument))]
    pub threshold: Option<FloatOrInt<0, { i32::MAX }>>,
    #[knuffel(child)]
    pub grow: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub grow_speed: Option<FloatOrInt<0, { i32::MAX }>>,
}

impl MergeWith<ShakeToEnlargePart> for ShakeToEnlarge {
    fn merge_with(&mut self, part: &ShakeToEnlargePart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge!((self, part), zoom_factor, threshold, grow_speed, grow);
        merge_clone!((self, part), hold_duration_ms);
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            xcursor_theme: String::from("default"),
            xcursor_size: 24,
            hide_when_typing: false,
            hide_after_inactive_ms: None,
            shake_to_enlarge: Some(ShakeToEnlarge::default()),
        }
    }
}

#[derive(knuffel::Decode, Debug, PartialEq)]
pub struct CursorPart {
    #[knuffel(child, unwrap(argument))]
    pub xcursor_theme: Option<String>,
    #[knuffel(child, unwrap(argument))]
    pub xcursor_size: Option<u8>,
    #[knuffel(child)]
    pub hide_when_typing: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub hide_after_inactive_ms: Option<u32>,
    #[knuffel(child)]
    pub shake_to_enlarge: Option<ShakeToEnlargePart>,
}

impl MergeWith<CursorPart> for Cursor {
    fn merge_with(&mut self, part: &CursorPart) {
        merge_clone!((self, part), xcursor_theme, xcursor_size);
        merge!((self, part), hide_when_typing);
        merge_clone_opt!((self, part), hide_after_inactive_ms);
        if let Some(x) = &part.shake_to_enlarge {
            if let Some(s) = &mut self.shake_to_enlarge {
                s.merge_with(x);
            } else {
                self.shake_to_enlarge = Some(ShakeToEnlarge::from_part(x));
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 光标特效配置（Cursor Effects）
//
// 参照 BASpark ConfigManager.cs:65-90 的字段与默认值，1:1 翻译为 niri 配置项。
// 此节点在 config.kdl 里写作 `cursor-effect { ... }`。
// ─────────────────────────────────────────────────────────────────────────────

/// 粒子颜色，BASpark 风格的逗号分隔 RGB 字符串（如 `"45,175,255"`）。
/// 亦接受 CSS 颜色（`#2dafff`、`rgb(45, 175, 255)` 等）。仿 appearance::Color
/// 手写 knuffel::Decode，取首个字符串实参走 FromStr。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleColor(pub [u8; 3]);

impl Default for ParticleColor {
    fn default() -> Self {
        // BASpark 默认 ParticleColor = "45,175,255"
        Self([45, 175, 255])
    }
}

impl std::str::FromStr for ParticleColor {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        // BASpark 风格：三个以逗号分隔的 0-255 整数（含 `rgb(...)`)。
        let cleaned: &str = s
            .trim_start_matches("rgb(")
            .trim_start_matches("rgba(")
            .trim_end_matches(')');
        let parts: Vec<&str> = cleaned.split(',').map(|p| p.trim()).collect();
        if parts.len() == 3 {
            let parse = |x: &str| -> Result<u8, String> {
                x.parse::<u8>().map_err(|_| format!("invalid color component `{x}`"))
            };
            return Ok(Self([parse(parts[0])?, parse(parts[1])?, parse(parts[2])?]));
        }
        // 回退：CSS 颜色解析。
        let c = csscolorparser::parse(s).map_err(|e| format!("invalid color `{s}`: {e}"))?;
        let c = c.clamp();
        Ok(Self([
            (c.r * 255.0).round() as u8,
            (c.g * 255.0).round() as u8,
            (c.b * 255.0).round() as u8,
        ]))
    }
}

impl<S> knuffel::Decode<S> for ParticleColor
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, knuffel::errors::DecodeError<S>> {
        if let Some(type_name) = &node.type_name {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }
        let mut iter_args = node.arguments.iter();
        let val = iter_args
            .next()
            .ok_or_else(|| knuffel::errors::DecodeError::missing(node, "a color string argument is required"))?;
        if let Some(typ) = &val.type_name {
            ctx.emit_error(knuffel::errors::DecodeError::TypeName {
                span: typ.span().clone(),
                found: Some((**typ).clone()),
                expected: knuffel::errors::ExpectedType::no_type(),
                rust_type: "str",
            });
        }
        let rv = match *val.literal {
            knuffel::ast::Literal::String(ref s) => {
                ParticleColor::from_str(s)
                    .map_err(|e| knuffel::errors::DecodeError::conversion(&val.literal, e))
            }
            _ => Err(knuffel::errors::DecodeError::unexpected(
                &val.literal,
                "argument",
                "expected a string color argument",
            )),
        }?;
        if let Some(val) = iter_args.next() {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                &val.literal,
                "argument",
                "only one color argument expected",
            ));
        }
        for child in node.children() {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                child,
                "node",
                "no child nodes expected for a color argument",
            ));
        }
        for name in node.properties.keys() {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                name,
                "property",
                "no properties expected for a color argument",
            ));
        }
        Ok(rv)
    }
}

impl From<ParticleColor> for Color {
    fn from(c: ParticleColor) -> Self {
        Color::from_rgba8_unpremul(c.0[0], c.0[1], c.0[2], 255)
    }
}

/// BASpark ClickTriggerType：左键/右键/二者皆触发。
///
/// 配置写作 `click-trigger "left"` / `"right"` / `"both"`，
/// 亦接受数字 `"0"`/`"1"`/`"2"`（字符串形）。与 `ParticleColor` 同采
/// 手写 knuffel::Decode（取首个字符串实参走 FromStr）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClickTriggerType {
    #[default]
    Left,
    Right,
    Both,
}

impl std::str::FromStr for ClickTriggerType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "left" | "Left" | "LEFT" | "0" => Ok(Self::Left),
            "right" | "Right" | "RIGHT" | "1" => Ok(Self::Right),
            "both" | "Both" | "BOTH" | "2" => Ok(Self::Both),
            other => Err(format!(
                "invalid click-trigger `{other}`: expected left/right/both or 0/1/2"
            )),
        }
    }
}

impl<S> knuffel::Decode<S> for ClickTriggerType
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, knuffel::errors::DecodeError<S>> {
        if let Some(type_name) = &node.type_name {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }
        let mut iter_args = node.arguments.iter();
        let val = iter_args
            .next()
            .ok_or_else(|| knuffel::errors::DecodeError::missing(node, "a click-trigger argument is required"))?;
        if let Some(typ) = &val.type_name {
            ctx.emit_error(knuffel::errors::DecodeError::TypeName {
                span: typ.span().clone(),
                found: Some((**typ).clone()),
                expected: knuffel::errors::ExpectedType::no_type(),
                rust_type: "str",
            });
        }
        let rv = match *val.literal {
            knuffel::ast::Literal::String(ref s) => {
                ClickTriggerType::from_str(s)
                    .map_err(|e| knuffel::errors::DecodeError::conversion(&val.literal, e))
            }
            _ => Err(knuffel::errors::DecodeError::unexpected(
                &val.literal,
                "argument",
                "expected a string click-trigger argument",
            )),
        }?;
        if let Some(val) = iter_args.next() {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                &val.literal,
                "argument",
                "only one click-trigger argument expected",
            ));
        }
        for child in node.children() {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                child,
                "node",
                "no child nodes expected for a click-trigger argument",
            ));
        }
        for name in node.properties.keys() {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                name,
                "property",
                "no properties expected for a click-trigger argument",
            ));
        }
        Ok(rv)
    }
}

/// 光标特效配置。字段名与默认值对应 BASpark ConfigManager.cs:65-90。
#[derive(Debug, Clone, PartialEq)]
pub struct CursorEffect {
    /// IsEffectEnabled（总开关，默认 true）。写 `enabled false` 关闭。
    pub enabled: bool,
    /// EffectScale（默认 1.5）。
    pub scale: f64,
    /// EffectOpacity（默认 1.0）。
    pub opacity: f64,
    /// ParticleColor（默认 "45,175,255"）。
    pub color: ParticleColor,
    /// UseLinkedAnimationSpeed（默认 true：trail/click 都跟随 effect_speed）。
    pub use_linked_animation_speed: bool,
    /// EffectSpeed（默认 1.0，动画速度基准）。
    pub effect_speed: f64,
    /// TrailAnimationSpeed（默认 1.0，use_linked 时被忽略）。
    pub trail_speed: f64,
    /// ClickAnimationSpeed（默认 1.0，use_linked 时被忽略）。
    pub click_speed: f64,
    /// TrailRefreshRate（默认 40，拖尾采样 Hz）。
    pub trail_refresh_rate: u32,
    /// EnableAlwaysTrailEffect（默认 false：仅按下时拖尾）。
    pub enable_always_trail: bool,
    /// ApplyCurveDraw（默认 false）。
    pub apply_curve_draw: bool,
    /// EnableMiddleClickTrigger（默认 false）。
    pub enable_middle_click_trigger: bool,
    /// ClickTriggerType（默认 Left）。
    pub click_trigger: ClickTriggerType,
    /// HideInFullscreen（默认 true：全屏下单屏隐藏）。
    pub hide_in_fullscreen: bool,
    /// ShowEffectOnDesktop（默认 true：桌面单屏显示）。
    pub show_on_desktop: bool,
    /// IsTouchscreenMode（默认 false）。
    pub touch_mode: bool,
}

impl Default for CursorEffect {
    fn default() -> Self {
        Self {
            enabled: true,
            scale: 1.5,
            opacity: 1.0,
            color: ParticleColor::default(),
            use_linked_animation_speed: true,
            effect_speed: 1.0,
            trail_speed: 1.0,
            click_speed: 1.0,
            trail_refresh_rate: 40,
            enable_always_trail: false,
            apply_curve_draw: false,
            enable_middle_click_trigger: false,
            click_trigger: ClickTriggerType::Left,
            hide_in_fullscreen: true,
            show_on_desktop: true,
            touch_mode: false,
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct CursorEffectPart {
    /// `enabled`（裸值=true 开启）或 `enabled false`（关闭）。
    #[knuffel(child)]
    pub enabled: Option<Flag>,
    /// `color "45,175,255"` 或 `color "#2dafff"`。
    #[knuffel(child)]
    pub color: Option<ParticleColor>,
    /// `use-linked-animation-speed false` 关闭链式速度。
    #[knuffel(child)]
    pub use_linked_animation_speed: Option<Flag>,
    #[knuffel(child)]
    pub enable_always_trail: Option<Flag>,
    #[knuffel(child)]
    pub apply_curve_draw: Option<Flag>,
    #[knuffel(child)]
    pub enable_middle_click_trigger: Option<Flag>,
    #[knuffel(child)]
    pub hide_in_fullscreen: Option<Flag>,
    #[knuffel(child)]
    pub show_on_desktop: Option<Flag>,
    #[knuffel(child)]
    pub touch_mode: Option<Flag>,
    #[knuffel(child, unwrap(argument))]
    pub scale: Option<FloatOrInt<0, 65536>>,
    #[knuffel(child, unwrap(argument))]
    pub opacity: Option<FloatOrInt<0, 65536>>,
    #[knuffel(child, unwrap(argument))]
    pub effect_speed: Option<FloatOrInt<0, { i32::MAX }>>,
    #[knuffel(child, unwrap(argument))]
    pub trail_speed: Option<FloatOrInt<0, { i32::MAX }>>,
    #[knuffel(child, unwrap(argument))]
    pub click_speed: Option<FloatOrInt<0, { i32::MAX }>>,
    #[knuffel(child, unwrap(argument))]
    pub trail_refresh_rate: Option<u32>,
    /// `click-trigger "left"` / `"right"` / `"both"`（或 `"0"`/`"1"`/`"2"`，字符串形）。
    #[knuffel(child)]
    pub click_trigger: Option<ClickTriggerType>,
}

impl MergeWith<CursorEffectPart> for CursorEffect {
    fn merge_with(&mut self, part: &CursorEffectPart) {
        merge!((self, part), enabled);
        merge!(
            (self, part),
            use_linked_animation_speed,
            enable_always_trail,
            apply_curve_draw,
            enable_middle_click_trigger,
            hide_in_fullscreen,
            show_on_desktop,
            touch_mode
        );
        // f64 ← Option<FloatOrInt<_>>（MergeWith<FloatOrInt> for f64 存在）。
        merge!((self, part), scale, opacity, effect_speed, trail_speed, click_speed);
        // u32 ← Option<u32> / copy types，直接覆盖。
        if let Some(x) = &part.trail_refresh_rate {
            self.trail_refresh_rate = *x;
        }
        if let Some(x) = &part.color {
            self.color = *x;
        }
        if let Some(x) = &part.click_trigger {
            self.click_trigger = *x;
        }
    }
}


#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct ScreenshotPath(#[knuffel(argument)] pub Option<String>);

impl Default for ScreenshotPath {
    fn default() -> Self {
        Self(Some(String::from(
            "~/Pictures/Screenshots/Screenshot from %Y-%m-%d %H-%M-%S.png",
        )))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyOverlay {
    pub skip_at_startup: bool,
    pub hide_not_bound: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HotkeyOverlayPart {
    #[knuffel(child)]
    pub skip_at_startup: Option<Flag>,
    #[knuffel(child)]
    pub hide_not_bound: Option<Flag>,
}

impl MergeWith<HotkeyOverlayPart> for HotkeyOverlay {
    fn merge_with(&mut self, part: &HotkeyOverlayPart) {
        merge!((self, part), skip_at_startup, hide_not_bound);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigNotification {
    pub disable_failed: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConfigNotificationPart {
    #[knuffel(child)]
    pub disable_failed: Option<Flag>,
}

impl MergeWith<ConfigNotificationPart> for ConfigNotification {
    fn merge_with(&mut self, part: &ConfigNotificationPart) {
        merge!((self, part), disable_failed);
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Clipboard {
    pub disable_primary: bool,
}

#[derive(knuffel::Decode, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardPart {
    #[knuffel(child)]
    pub disable_primary: Option<Flag>,
}

impl MergeWith<ClipboardPart> for Clipboard {
    fn merge_with(&mut self, part: &ClipboardPart) {
        merge!((self, part), disable_primary);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Magnifier {
    pub off: bool,
    pub zoom_factor: f64,
    pub track_cursor: bool,
    pub scale_cursor: bool,
}

impl Default for Magnifier {
    fn default() -> Self {
        Self {
            off: false,
            zoom_factor: 2.0,
            track_cursor: true,
            scale_cursor: true,
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub struct MagnifierPart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child, unwrap(argument))]
    pub zoom_factor: Option<FloatOrInt<0, { i32::MAX }>>,
    #[knuffel(child)]
    pub track_cursor: Option<Flag>,
    #[knuffel(child)]
    pub scale_cursor: Option<Flag>,
}

impl MergeWith<MagnifierPart> for Magnifier {
    fn merge_with(&mut self, part: &MagnifierPart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }
        merge!((self, part), zoom_factor, track_cursor, scale_cursor);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overview {
    pub zoom: f64,
    pub backdrop_color: Color,
    pub workspace_shadow: WorkspaceShadow,
}

impl Default for Overview {
    fn default() -> Self {
        Self {
            zoom: 0.5,
            backdrop_color: DEFAULT_BACKDROP_COLOR,
            workspace_shadow: WorkspaceShadow::default(),
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct OverviewPart {
    #[knuffel(child, unwrap(argument))]
    pub zoom: Option<FloatOrInt<0, 1>>,
    #[knuffel(child)]
    pub backdrop_color: Option<Color>,
    #[knuffel(child)]
    pub workspace_shadow: Option<WorkspaceShadowPart>,
}

impl MergeWith<OverviewPart> for Overview {
    fn merge_with(&mut self, part: &OverviewPart) {
        merge!((self, part), zoom, workspace_shadow);
        merge_clone!((self, part), backdrop_color);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridOverview {
    pub gap: f64,
    pub padding: GridOverviewPadding,
    pub min_scale: f64,
    pub focused_column_scale: f64,
    pub grid_all_monitors: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridOverviewPadding {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

impl GridOverviewPadding {
    pub fn uniform(value: f64) -> Self {
        Self {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }
}

impl Default for GridOverviewPadding {
    fn default() -> Self {
        Self::uniform(80.)
    }
}

impl Default for GridOverview {
    fn default() -> Self {
        Self {
            gap: 16.,
            padding: GridOverviewPadding::default(),
            min_scale: 0.08,
            focused_column_scale: 1.04,
            grid_all_monitors: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridOverviewPaddingPart {
    Uniform(f64),
    Sides {
        left: Option<f64>,
        right: Option<f64>,
        top: Option<f64>,
        bottom: Option<f64>,
    },
}

impl GridOverviewPaddingPart {
    fn merge_into(&self, padding: &mut GridOverviewPadding) {
        match *self {
            Self::Uniform(value) => *padding = GridOverviewPadding::uniform(value),
            Self::Sides {
                left,
                right,
                top,
                bottom,
            } => {
                if let Some(left) = left {
                    padding.left = left;
                }
                if let Some(right) = right {
                    padding.right = right;
                }
                if let Some(top) = top {
                    padding.top = top;
                }
                if let Some(bottom) = bottom {
                    padding.bottom = bottom;
                }
            }
        }
    }
}

impl<S: knuffel::traits::ErrorSpan> knuffel::Decode<S> for GridOverviewPaddingPart {
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, knuffel::errors::DecodeError<S>> {
        let mut iter_args = node.arguments.iter();
        if let Some(val) = iter_args.next() {
            let value: FloatOrInt<0, { i32::MAX }> =
                knuffel::traits::DecodeScalar::decode(val, ctx)?;

            if let Some(val) = iter_args.next() {
                ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                    &val.literal,
                    "argument",
                    "unexpected argument",
                ));
            }
            for child in node.children() {
                ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                    child,
                    "node",
                    "no child nodes expected for `padding` with an argument",
                ));
            }
            for name in node.properties.keys() {
                ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                    name,
                    "property",
                    "no properties expected for this node",
                ));
            }

            return Ok(Self::Uniform(value.0));
        }

        let mut left = None;
        let mut right = None;
        let mut top = None;
        let mut bottom = None;

        for child in node.children() {
            let value: FloatOrInt<0, { i32::MAX }> = match &**child.node_name {
                "left" | "right" | "top" | "bottom" => {
                    parse_arg_node(&child.node_name, child, ctx)?
                }
                name => {
                    ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                        child,
                        "node",
                        format!("unknown padding property `{name}`"),
                    ));
                    continue;
                }
            };

            match &**child.node_name {
                "left" => left = Some(value.0),
                "right" => right = Some(value.0),
                "top" => top = Some(value.0),
                "bottom" => bottom = Some(value.0),
                _ => unreachable!(),
            }
        }

        for name in node.properties.keys() {
            ctx.emit_error(knuffel::errors::DecodeError::unexpected(
                name,
                "property",
                "no properties expected for this node",
            ));
        }

        Ok(Self::Sides {
            left,
            right,
            top,
            bottom,
        })
    }
}

#[derive(knuffel::Decode, Debug, Clone, Copy, PartialEq)]
pub struct GridOverviewPart {
    #[knuffel(child, unwrap(argument))]
    pub gap: Option<FloatOrInt<0, { i32::MAX }>>,
    #[knuffel(child)]
    pub padding: Option<GridOverviewPaddingPart>,
    #[knuffel(child, unwrap(argument))]
    pub min_scale: Option<FloatOrInt<0, 1>>,
    #[knuffel(child, unwrap(argument))]
    pub focused_column_scale: Option<FloatOrInt<1, 2>>,
    #[knuffel(child, unwrap(argument))]
    pub grid_all_monitors: Option<bool>,
}

impl MergeWith<GridOverviewPart> for GridOverview {
    fn merge_with(&mut self, part: &GridOverviewPart) {
        if let Some(gap) = &part.gap {
            self.gap = gap.0;
        }
        if let Some(padding) = &part.padding {
            padding.merge_into(&mut self.padding);
        }
        if let Some(min_scale) = &part.min_scale {
            self.min_scale = min_scale.0;
        }
        if let Some(focused_column_scale) = &part.focused_column_scale {
            self.focused_column_scale = focused_column_scale.0;
        }
        if let Some(grid_all_monitors) = part.grid_all_monitors {
            self.grid_all_monitors = grid_all_monitors;
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, Clone, PartialEq, Eq)]
pub struct Environment(#[knuffel(children)] pub Vec<EnvironmentVariable>);

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentVariable {
    #[knuffel(node_name)]
    pub name: String,
    #[knuffel(argument)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XwaylandSatellite {
    pub off: bool,
    pub path: String,
}

impl Default for XwaylandSatellite {
    fn default() -> Self {
        Self {
            off: false,
            path: String::from("xwayland-satellite"),
        }
    }
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq, Eq)]
pub struct XwaylandSatellitePart {
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child, unwrap(argument))]
    pub path: Option<String>,
}

impl MergeWith<XwaylandSatellitePart> for XwaylandSatellite {
    fn merge_with(&mut self, part: &XwaylandSatellitePart) {
        self.off |= part.off;
        if part.on {
            self.off = false;
        }

        merge_clone!((self, part), path);
    }
}
