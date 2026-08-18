//! 光标特效纯逻辑状态机（Cursor Effects pure-logic state machine）。
//!
//! BASpark `reference/baspark/index.html` 的 `MouseSpark` 物理/动画逻辑 1:1 翻译为纯 Rust。
//! 无 GLES、无 niri 渲染依赖，可在单元测试中独立驱动。
//!
//! 行号引用对应 BASpark `index.html`：常量 @ 34-52，createEffects @ 199-260，
//! mousemove @ 107-145，updateTrail @ 288-393，updateWaves @ 404-475，
//! updateSparks @ 490-525，animationLoops/hasWork @ 670-696。
//! 配置默认值对应 `reference/baspark/ConfigManager.cs:65-90`。

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};
use std::time::Instant;

// ────────────────────────── 硬参数常量（直照 BASpark index.html:35-52）──────────────────────────

/// BASpark index.html:35  FILLED_CIRCLE_CFG
pub mod filled_cfg {
    pub const R_ADD_RATE: f32 = 26.0;
    pub const MAX_LIFE: f32 = 16.0;
    /// 淡出窗口延长系数：半径/运动仍由 MAX_LIFE 驱动，淡出 alpha 用
    /// `life / (MAX_LIFE * FADE_EXTEND)` → 圆盘膨胀速度不变，消失更慢、
    /// 各格错开的淡出时间更明显。
    pub const FADE_EXTEND: f32 = 4.0;
}

/// BASpark index.html:36-45  RINGS_ANIM_CFG
pub mod rings_cfg {
    pub const RS_LIST: [f32; 3] = [0.0, 0.08, 0.1];
    pub const R_ROUND_RATE_LIST: [f32; 4] = [0.0, 1.0, 1.5, 2.0];
    pub const LEN: f32 = 1.1 * std::f32::consts::PI;
    pub const MAX_LIFE: f32 = 23.0;
    /// 同上：环的淡出/收缩更慢，位置运动不变；延长后各字符独立消失时间更明显。
    pub const FADE_EXTEND: f32 = 4.0;
    pub const SEG_NUM: usize = 10;
    pub const MIN_W: f32 = 0.4;
    pub const MAX_W: f32 = 3.3;
    pub const LEN_STOP_ADD_POINT: f32 = 0.1;
    pub const LEN_START_DIM_POINT: f32 = 0.4;
}

/// BASpark index.html:47-52  CREATE_CLICK_CFG
pub mod create_click_cfg {
    pub const RINGS_RS_LIST: [f32; 3] = [0.0, 0.03, 0.06];
    pub const RINGS_R_ROUND_RATE_LIST: [f32; 4] = [0.0, 1.0, 1.5, 2.0];
    pub const RINGS_LEN: f32 = 1.1 * std::f32::consts::PI;
    pub const SPARKS_COUNT: usize = 8;
}

/// BASpark index.html:66-68  animationLoops 帧率归一参数
pub const BASE_FRAME_MS: f32 = 1000.0 / 60.0;
pub const MAX_DELTA_MS: f32 = 100.0;

// ────────────────────────── 随机源 ──────────────────────────

/// 注入式随机源：生产用 fastrand（非确定性），测试用 SequentialRng（确定性）。
pub trait EffectRng {
    /// 返回 [0, 1) 的 f32。
    fn f32(&mut self) -> f32;
}

/// 生产随机源：基于 fastrand（niri 已依赖 fastrand 2.4.1）。
#[derive(Debug, Default)]
pub struct FastrandRng;
impl EffectRng for FastrandRng {
    #[inline]
    fn f32(&mut self) -> f32 {
        fastrand::f32()
    }
}

// ────────────────────────── 数据结构（对应 BASpark wave/ring/spark/trail）──────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct RingSeg {
    pub off: f32,
    pub len: f32,
    pub r_round_rate: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct Ring {
    pub ang: f32,
    pub rs: f32,
    pub segs: [RingSeg; 2],
}

#[derive(Debug, Clone, Copy)]
pub struct Wave {
    pub x: f32,
    pub y: f32,
    pub r: f32,
    pub life: f32,
    pub ring: Ring,
    /// 每次点击随机生成的种子：让环/圆盘每个字符的速度、半径、淡出时间
    /// 每次点击都不同（否则由弧位置 k 派生 → 每次点击几乎一样）。
    pub seed: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Fragment {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub rot: f32,
    pub rs: f32,
    pub s: f32,
    pub a: f32,
    pub f: f32,
    pub from_click: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct TrailPoint {
    pub x: f32,
    pub y: f32,
    pub life: f32,
    /// 稳定标识（创建时递增分配），用作字符闪烁的随机种子——保证字符只在
    /// 寿命期内的固定时间点变化，而不是随队列 index 位移而抖动。
    pub id: u64,
}

/// BASpark ClickTriggerType（ConfigManager.cs:89）：0=左, 1=右, 2=左右。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickTrigger {
    Left,
    Right,
    Both,
}

/// BASpark index.html:55-57  ringsEndColorFromRgb：c → (c + 255*2) / 3。
pub fn rings_end_color_from_rgb(rgb: [u8; 3]) -> [f32; 3] {
    let map = |c: u8| (c as f32 + 255.0 * 2.0) / 3.0;
    [map(rgb[0]), map(rgb[1]), map(rgb[2])]
}

// ────────────────────────── 状态机 ──────────────────────────

/// 光标特效运行时状态。BASpark `MouseSpark`（index.html:59-77）的 Rust 对应。
#[derive(Debug)]
pub struct CursorEffectState {
    pub enabled: bool,
    pub color: [u8; 3],
    pub rings_start_color: [f32; 3],
    pub rings_end_color: [f32; 3],
    pub scale: f32,
    pub opacity: f32,
    pub trail_speed: f32,
    pub click_speed: f32,
    pub max_trail: usize,
    pub trail_refresh_hz: u32,
    pub persistent_trail: bool,
    pub apply_curve_draw: bool,
    pub touch_mode: bool,
    pub click_trigger: ClickTrigger,
    pub middle_click_trigger: bool,
    pub hide_in_fullscreen: bool,
    pub show_on_desktop: bool,

    pub waves: Vec<Wave>,
    pub sparks: Vec<Fragment>,
    pub trail: VecDeque<TrailPoint>,
    pub last_pos: Option<(f32, f32)>,
    pub is_down: bool,
    pub last_frame_time: Instant,
    pub last_trail_emit: Instant,
    /// 进程起点（字符模式用相对时间做 ~200ms 周期的闪烁节拍）。
    pub instant0: Instant,
    /// 拖尾点稳定 id 分配器。
    pub trail_serial: u64,
}

impl Default for CursorEffectState {
    fn default() -> Self {
        Self::new()
    }
}

impl CursorEffectState {
    /// BASpark index.html:62-77 构造 + ConfigManager.cs:65-90 默认值。
    pub fn new() -> Self {
        let color = [45, 175, 255]; // ParticleColor 默认
        let now = Instant::now();
        Self {
            enabled: true, // IsEffectEnabled
            color,
            rings_start_color: [250.0, 252.0, 252.0], // index.html:76
            rings_end_color: rings_end_color_from_rgb(color), // index.html:77
            scale: 1.5,      // EffectScale
            opacity: 1.0,    // EffectOpacity
            trail_speed: 1.0, // EffectSpeed (link 默认)
            click_speed: 1.0,
            max_trail: 320,  // 密集 ~2px 采样需更多点才能维持足够拖尾长度（320 点 ≈ 640px）
            trail_refresh_hz: 40, // TrailRefreshRate
            persistent_trail: false, // EnableAlwaysTrailEffect
            apply_curve_draw: false, // ApplyCurveDraw
            touch_mode: false, // IsTouchscreenMode
            click_trigger: ClickTrigger::Left, // ClickTriggerType=0
            middle_click_trigger: false, // EnableMiddleClickTrigger
            hide_in_fullscreen: true, // HideInFullscreen
            show_on_desktop: true, // ShowEffectOnDesktop
            waves: Vec::with_capacity(8),
            sparks: Vec::with_capacity(32),
            trail: VecDeque::with_capacity(16),
            last_pos: None,
            is_down: false,
            last_frame_time: now,
            last_trail_emit: now,
            instant0: now,
            trail_serial: 0,
        }
    }

    /// BASpark index.html:261-264  alpha(value) = clamp(value*opacity, 0, 1)
    #[inline]
    pub fn alpha(&self, value: f32) -> f32 {
        let v = value.clamp(0.0, 1.0) * self.opacity;
        v.clamp(0.0, 1.0)
    }

    /// 清空所有粒子并复位即时状态（对应 BASpark clearEffects + toggle-off 行为）。
    pub fn clear(&mut self) {
        self.waves.clear();
        self.sparks.clear();
        self.trail.clear();
        self.is_down = false;
        self.last_pos = None;
        self.last_frame_time = Instant::now();
    }

    /// BASpark index.html:404-406  weightProp(t) = min(2 - |4*(t-0.5)|, 1)
    #[inline]
    pub fn weight_prop(t: f32) -> f32 {
        (2.0 - (4.0 * (t - 0.5)).abs()).min(1.0)
    }

    /// BASpark index.html:419-426  ringRgbAt(r): 在 ringsStart→ringsEnd 插值，t = min(1.2*r,1)
    pub fn ring_rgb_at(&self, r_prog: f32) -> [f32; 3] {
        let t = (1.2 * r_prog).min(1.0);
        let lerp = |s: f32, e: f32| (s * (1.0 - t) + e * t).round();
        [
            lerp(self.rings_start_color[0], self.rings_end_color[0]),
            lerp(self.rings_start_color[1], self.rings_end_color[1]),
            lerp(self.rings_start_color[2], self.rings_end_color[2]),
        ]
    }

    /// BASpark index.html:427  getAlpha(r) = min(1.1 - 0.3*r, 1)
    #[inline]
    pub fn ring_alpha(&self, r_prog: f32) -> f32 {
        (1.1 - 0.3 * r_prog).min(1.0)
    }
}

// ────────────────────────── 事件注入（对应 BASpark mousedown/mousemove/mouseup）──────────────────────────

impl ClickTrigger {
    /// 是否接受此按键事件。`is_left`/`is_right`/`is_middle` 三个布尔互斥（由调用方从按钮 code 推得）。
    pub fn accepts(&self, is_left: bool, is_right: bool) -> bool {
        match self {
            ClickTrigger::Left => is_left,
            ClickTrigger::Right => is_right,
            ClickTrigger::Both => is_left || is_right,
        }
    }
}

impl CursorEffectState {
    /// BASpark index.html:199-260  createEffects —— 1:1 翻译。
    pub fn create_effects<R: EffectRng>(&mut self, x: f32, y: f32, rng: &mut R) {
        let rr = create_click_cfg::RINGS_R_ROUND_RATE_LIST;
        let pick = |rng: &mut R, slice: &[f32]| -> f32 {
            let i = (rng.f32() * slice.len() as f32) as usize;
            slice[i.min(slice.len() - 1)]
        };
        let wave_seed = ((rng.f32() as f64 * 1e9) as u64) ^ (((rng.f32() as f64 * 1e9) as u64) << 32);
        self.waves.push(Wave {
            x,
            y,
            r: 0.0,
            life: 0.0,
            seed: wave_seed,
            ring: Ring {
                ang: rng.f32() * TAU,
                rs: pick(rng, &create_click_cfg::RINGS_RS_LIST),
                segs: [
                    RingSeg {
                        off: 0.0,
                        len: create_click_cfg::RINGS_LEN,
                        r_round_rate: pick(rng, &rr),
                    },
                    RingSeg {
                        off: (rng.f32() * 3.0 - 1.5) * PI,
                        len: create_click_cfg::RINGS_LEN,
                        r_round_rate: pick(rng, &rr),
                    },
                ],
            },
        });
        let speed_adjust = self.scale / 1.5; // index.html:247
        for _ in 0..create_click_cfg::SPARKS_COUNT {
            let a = rng.f32() * TAU;
            let speed = (4.8 + rng.f32() * 2.0) * speed_adjust; // index.html:249
            self.sparks.push(Fragment {
                x,
                y,
                vx: a.cos() * speed,
                vy: a.sin() * speed,
                rot: rng.f32() * TAU,
                rs: (rng.f32() - 0.5) * 0.28,
                s: (4.0 + rng.f32() * 3.0) * self.scale, // index.html:254
                a: 1.0,
                f: 0.9,
                from_click: true,
            });
        }
    }

    /// BASpark index.html:107-145  mousemove 拖尾 + 0.3 概率 spawn 拖尾碎片 —— 1:1 翻译。
    /// 节流（trailRefreshRate）由调用方在 call 前做，本方法只推进逻辑（见计划 §1.5）。
    pub fn on_move<R: EffectRng>(&mut self, nx: f32, ny: f32, _now: Instant, rng: &mut R) {
        if !self.is_down && !self.persistent_trail {
            return;
        }
        let p = (nx, ny);
        let Some(prev) = self.last_pos else {
            self.last_pos = Some(p);
            return;
        };
        self.last_pos = Some(p);

        // 拖尾采样参考点 = 已入队的最后一个点（trail 为空时退回 last_pos）。
        // 这样拖尾点沿路径保持 ~2px 间距，与指针事件采样率无关。若以"单个事件"
        // 的 last_pos 为参考，高轮询率鼠标快速晃动时单事件位移常 <2px（转向点
        // 附近速度低），导致只在高速过中点附近 push 点、转向点处断档 → 点聚集
        // 在中点而头在极值 → 拖尾退化成"锁定鼠标"的笔直直线；同时点稀疏 → 折线
        // 以长直线弦连接采样点，快速曲线运动下锯齿明显。
        let (rx, ry) = self.trail.back().map(|tp| (tp.x, tp.y)).unwrap_or(prev);
        let d = ((p.0 - rx).powi(2) + (p.1 - ry).powi(2)).sqrt();
        if d > 2.0 {
            // 对移动段按 ~2px 细分，即使单事件大跳变也生成密集折线点，保证拖尾
            // 沿真实路径平滑跟踪曲线（消除长弦带来的锯齿）。
            let steps = ((d / 2.0).ceil().max(1.0)) as usize;
            for k in 1..=steps {
                let t = k as f32 / steps as f32;
                self.trail_serial = self.trail_serial.wrapping_add(1);
                self.trail.push_back(TrailPoint {
                    x: rx + (p.0 - rx) * t,
                    y: ry + (p.1 - ry) * t,
                    life: 1.0,
                    id: self.trail_serial,
                });
            }
            while self.trail.len() > self.max_trail {
                self.trail.pop_front();
            }
            if rng.f32() < 0.3 {
                let a = rng.f32() * TAU;
                let sa = self.scale / 1.5;
                self.sparks.push(Fragment {
                    x: p.0 + a.cos() * 10.0 * self.scale, // index.html:130
                    y: p.1 + a.sin() * 10.0 * self.scale,
                    vx: a.cos() * 1.3 * sa,
                    vy: a.sin() * 1.3 * sa,
                    rot: rng.f32() * TAU,
                    rs: 0.16,
                    s: 9.0 * self.scale,
                    a: 0.7,
                    f: 0.95,
                    from_click: false,
                });
            }
        }
        self.last_pos = Some(p);
    }
}

// ────────────────────────── 每帧推进（对应 BASpark animationLoops + update*）──────────────────────────

impl CursorEffectState {
    /// BASpark index.html:681  hasWork：无粒子即冬眠。
    pub fn has_work(&self) -> bool {
        !self.waves.is_empty() || !self.sparks.is_empty() || !self.trail.is_empty()
    }

    /// 推进一帧。返回本帧后是否仍有粒子存活（供调用方决定 redraw）。
    /// 对应 BASpark index.html:670-696 animationLoops + updateTrail/Waves/Sparks。
    pub fn advance(&mut self, now: Instant) -> bool {
        if !self.enabled {
            self.clear();
            return false;
        }
        if !self.has_work() {
            // index.html:692-696：静止时同步时间基准，避免首帧 delta 顶到 maxDeltaMs。
            self.last_frame_time = now;
            return false;
        }
        let elapsed = now.saturating_duration_since(self.last_frame_time);
        let delta_ms = (elapsed.as_secs_f32() * 1000.0).min(MAX_DELTA_MS); // index.html:690
        self.last_frame_time = now;
        let base_scale = delta_ms / BASE_FRAME_MS; // index.html:691
        let trail_fs = base_scale * self.trail_speed; // index.html:698
        let click_fs = base_scale * self.click_speed; // index.html:699
        self.update_trail(trail_fs);
        self.update_waves(click_fs);
        self.update_sparks(click_fs, trail_fs);
        self.has_work()
    }

    /// BASpark index.html:288-321  updateTrail 寿命衰减（绘制在 render 层）。
    fn update_trail(&mut self, trail_fs: f32) {
        let n = self.trail.len();
        let base_decay = if self.persistent_trail {
            0.035
        } else if self.is_down {
            0.035
        } else {
            0.10
        }; // index.html:292-296（数值调慢：拖尾消失更慢，位置/速度不变）
        let base_decay = base_decay * trail_fs;
        let max_step = 0.42; // index.html:297
        let span = (n as f32 - 1.0).max(1.0);
        let mut i = n;
        while i > 0 {
            i -= 1;
            let along = if n > 1 { i as f32 / span } else { 1.0 }; // index.html:301
            let bias = 1.25 - 0.55 * along; // index.html:302
            let step = (base_decay * bias).min(max_step);
            let p = &mut self.trail[i];
            p.life -= step;
            if p.life <= 0.0 {
                self.trail.remove(i); // VecDeque::remove O(n)，max 16 可接受
            }
        }
    }

    /// BASpark index.html:404-475  updateWaves 寿命+filled 半径+ring 旋转+回收。
    /// 段几何（start/end/len/lineWidthMul）在 render 层按 ring_prog 重新计算（与
    /// _strokeRingSegment 同帧一起），state 层只保留寿命与旋转，weight_prop/ring_rgb_at/
    /// ring_alpha 作为纯函数供 render 调用。
    fn update_waves(&mut self, click_fs: f32) {
        let mut i = 0;
        while i < self.waves.len() {
            // 先在可变借用内完成全部计算，再决定回收，避免在借用中 swap_remove。
            let recycle = {
                let w = &mut self.waves[i];
                w.life += click_fs; // index.html:404-405
                let wave_prog = (w.life / filled_cfg::MAX_LIFE).min(1.0); // index.html:444（运动）
                let ease = 1.0 - (1.0 - wave_prog).powi(3); // index.html:407
                w.r = filled_cfg::R_ADD_RATE * self.scale * ease; // index.html:408
                w.ring.ang -= w.ring.rs * click_fs; // index.html:437
                // 回收等淡出完成（FADE_EXTEND 延长窗口），而非运动完成就回收。
                let fill_fade_done = w.life >= filled_cfg::MAX_LIFE * filled_cfg::FADE_EXTEND;
                let ring_fade_done = w.life >= rings_cfg::MAX_LIFE * rings_cfg::FADE_EXTEND;
                fill_fade_done && ring_fade_done
            };
            if recycle {
                self.waves.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// BASpark index.html:490-525  updateSparks —— 1:1 翻译（only 寿命/积分，绘制在 render 层）。
    fn update_sparks(&mut self, click_fs: f32, trail_fs: f32) {
        let mut i = 0;
        while i < self.sparks.len() {
            let recycle = {
                let s = &mut self.sparks[i];
                let fs = if s.from_click { click_fs } else { trail_fs }; // index.html:496
                s.x += s.vx * fs; // index.html:500
                s.y += s.vy * fs;
                s.vx *= s.f.powf(fs); // index.html:502
                s.vy *= s.f.powf(fs);
                s.rot += s.rs * fs; // index.html:504
                s.a -= 0.020 * fs; // index.html:505
                s.a <= 0.0
            };
            if recycle {
                self.sparks.swap_remove(i); // index.html:506
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 确定性随机桩：循环返回固定序列，便于断言粒子数与坐标。
    struct SequentialRng {
        seq: Vec<f32>,
        idx: usize,
    }
    impl SequentialRng {
        fn new(seq: &[f32]) -> Self {
            Self {
                seq: seq.to_vec(),
                idx: 0,
            }
        }
    }
    impl EffectRng for SequentialRng {
        fn f32(&mut self) -> f32 {
            let v = self.seq[self.idx % self.seq.len()];
            self.idx += 1;
            v
        }
    }

    #[test]
    fn end_color_formula_is_two_thirds_blend() {
        // index.html:55-57：c=45 → (45 + 510) / 3 = 185；c=175 → (175+510)/3 = 228.33…；c=255 → 255
        let e = rings_end_color_from_rgb([45, 175, 255]);
        assert!((e[0] - 185.0).abs() < 1e-3);
        assert!((e[1] - 228.333_3).abs() < 1e-3);
        assert!((e[2] - 255.0).abs() < 1e-3);
    }

    #[test]
    fn weight_prop_is_unit_bell_peak_at_half() {
        assert!((CursorEffectState::weight_prop(0.5) - 1.0).abs() < 1e-3);
        // 端点应为 0（|4*(0-0.5)|=2，2-2=0）
        assert!((CursorEffectState::weight_prop(0.0)).abs() < 1e-3);
        assert!((CursorEffectState::weight_prop(1.0)).abs() < 1e-3);
    }

    #[test]
    fn ring_alpha_clamped_to_one() {
        let s = CursorEffectState::new();
        assert!((s.ring_alpha(0.0) - 1.0).abs() < 1e-3);
        assert!((s.ring_alpha(1.0) - 0.8).abs() < 1e-3);
    }

    #[test]
    fn create_effects_makes_one_wave_and_four_sparks() {
        let mut s = CursorEffectState::new();
        let mut rng = SequentialRng::new(&[0.5; 8]);
        s.create_effects(10.0, 10.0, &mut rng);
        assert_eq!(s.waves.len(), 1);
        assert_eq!(s.sparks.len(), create_click_cfg::SPARKS_COUNT);
        assert!(s.has_work());
        // wave 初值
        let w = s.waves[0];
        assert_eq!((w.x, w.y), (10.0, 10.0));
        assert_eq!(w.r, 0.0);
        assert_eq!(w.life, 0.0);
        assert_eq!(w.ring.segs.len(), 2);
        // sparks 来自点击
        for sp in &s.sparks {
            assert!(sp.from_click);
            assert_eq!(sp.a, 1.0);
            assert_eq!(sp.f, 0.9);
        }
    }

    #[test]
    fn on_move_only_trails_when_down_or_persistent() {
        let mut s = CursorEffectState::new();
        let mut rng = SequentialRng::new(&[0.5; 8]);
        let now = Instant::now();
        s.is_down = false;
        s.persistent_trail = false;
        s.on_move(0.0, 0.0, now, &mut rng);
        assert!(s.trail.is_empty());
        s.is_down = true;
        s.on_move(10.0, 10.0, now, &mut rng); // last_pos 先被设
        s.on_move(60.0, 60.0, now, &mut rng); // 距离 70>2 → 沿段 ~2px 细分采样
        assert!(!s.trail.is_empty());
        assert!(s.trail.len() <= s.max_trail);
        // 拖尾点应密集覆盖 (10,10)→(60,60) 段，末端就是当前点
        let last = s.trail.back().unwrap();
        assert!((last.x - 60.0).abs() < 1e-3 && (last.y - 60.0).abs() < 1e-3);
    }

    #[test]
    fn trail_dense_sampling_follows_fast_shake_path() {
        // 回归：快速晃动鼠标时拖尾退化成"锁定鼠标"的笔直直线。
        // 高轮询率鼠标单事件位移常 <2px（转向点附近速度低），若以单事件 last_pos
        // 为参考，转向点附近几乎不采样 → 点聚集在中点而头在极值 → 成直线。
        // 修复：参考点改为 trail.back()（已入队最后一点）+ ~2px 细分，路径上保持
        // 密集采样。
        let mut s = CursorEffectState::new();
        let mut rng = SequentialRng::new(&[0.5; 64]);
        let now = Instant::now();
        s.is_down = true;
        s.on_move(0.0, 0.0, now, &mut rng); // 初始化 last_pos
        s.on_move(40.0, 0.0, now, &mut rng); // 快速段建立拖尾（参考点=trail.back()）
        assert!(!s.trail.is_empty());
        let mut x = 40.0;
        let mut dir = -1.0;
        let mut final_x = x;
        for _ in 0..400 {
            x += dir;
            if x < -40.0 {
                dir = 1.0;
            }
            if x > 40.0 {
                dir = -1.0;
            }
            final_x = x;
            s.on_move(x, 0.0, now, &mut rng); // 之后每事件仅 1px（高轮询率）
        }
        assert!(!s.trail.is_empty());
        let xs: Vec<f32> = s.trail.iter().map(|p| p.x).collect();
        for w in xs.windows(2) {
            assert!(
                (w[1] - w[0]).abs() <= 3.0 + 1e-3,
                "trail gap too large: {xs:?}"
            );
        }
        // 拖尾必须跟随晃动前进，而非卡在起点 seed 成一条直线
        let back = s.trail.back().unwrap().x;
        assert!(
            (back - final_x).abs() <= 3.0 + 1e-3,
            "trail stuck at seed (straight-line bug), back={back}, final={final_x}"
        );
    }

    #[test]
    fn advance_at_60hz_keeps_tail_alive() {
        let mut s = CursorEffectState::new();
        let mut rng = SequentialRng::new(&[0.5; 8]);
        let t0 = Instant::now();
        s.is_down = true;
        s.on_move(0.0, 0.0, t0, &mut rng);
        s.on_move(50.0, 50.0, t0, &mut rng);
        assert!(!s.trail.is_empty());
        let alive = s.advance(t0 + Duration::from_millis(16)); // ~1 帧 @ 60Hz
        assert!(alive);
    }

    #[test]
    fn sparks_die_by_alpha_within_enough_frames() {
        let mut s = CursorEffectState::new();
        let mut rng = SequentialRng::new(&[0.5; 8]);
        s.create_effects(0.0, 0.0, &mut rng);
        // a 每帧 -0.032*fs（fs≈1 @ 60Hz×speed=1）；约 32 帧耗尽
        let mut now = Instant::now();
        for _ in 0..200 {
            now += Duration::from_millis(16);
            s.advance(now);
        }
        assert_eq!(s.sparks.len(), 0, "sparks should all decay by alpha");
    }

    #[test]
    fn waves_recycle_when_both_progress_complete() {
        let mut s = CursorEffectState::new();
        let mut rng = SequentialRng::new(&[0.5; 8]);
        s.create_effects(0.0, 0.0, &mut rng);
        // ring max_life=23，filled max_life=16；运行 >23 帧应全部回收
        let mut now = Instant::now();
        for _ in 0..300 {
            now += Duration::from_millis(16);
            s.advance(now);
        }
        assert_eq!(s.waves.len(), 0, "wave should recycle after both progress hit 1");
    }

    #[test]
    fn disabled_clears_all_particles() {
        let mut s = CursorEffectState::new();
        let mut rng = SequentialRng::new(&[0.5; 8]);
        s.create_effects(0.0, 0.0, &mut rng);
        assert!(!s.sparks.is_empty());
        s.enabled = false;
        let alive = s.advance(Instant::now());
        assert!(!alive);
        assert_eq!(s.sparks.len(), 0);
        assert_eq!(s.waves.len(), 0);
        assert!(s.trail.is_empty());
    }

    #[test]
    fn has_work_false_resyncs_time_no_particles_drift() {
        let mut s = CursorEffectState::new();
        let t1 = Instant::now() + Duration::from_millis(5000);
        let alive = s.advance(t1); // 无粒子：同步时间基准
        assert!(!alive);
        assert_eq!(s.last_frame_time, t1, "time base resynced to now");
    }

    #[test]
    fn trail_cap_respected_at_max_trail() {
        let mut s = CursorEffectState::new();
        s.max_trail = 3;
        s.is_down = true;
        let mut rng = SequentialRng::new(&[1.0; 16]); // rng=1 → 不 spawn 拖尾碎片(<0.3 失败)
        let now = Instant::now();
        for i in 0..10 {
            s.on_move(i as f32 * 50.0, i as f32 * 50.0, now, &mut rng);
        }
        assert!(s.trail.len() <= s.max_trail, "trail capped at max_trail");
    }

    #[test]
    fn click_trigger_accepts_routing() {
        assert!(ClickTrigger::Left.accepts(true, false));
        assert!(!ClickTrigger::Left.accepts(false, true));
        assert!(ClickTrigger::Right.accepts(false, true));
        assert!(!ClickTrigger::Right.accepts(true, false));
        assert!(ClickTrigger::Both.accepts(true, false));
        assert!(ClickTrigger::Both.accepts(false, true));
    }
}
