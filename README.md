# Niri β 个人修改版

> ⚠️ **警告：这是个人向的自用分支，不是稳定发行版。**
>
> 基于 niri 的深度定制，只适合我自己的使用习惯和环境。与官方版本可能存在差异，
> 随时可能改动或删减，**不适合所有人**，请勿当作通用发行版使用。如果你只是想
> 要一个好用的 niri，请用官方版本。

## 简介

在 niri 官方的基础上集成了我感兴趣的若干功能，版本会不定期跟进更新。

已集成：

1. **Shorin-niri 的全部功能**（网格预览、放大镜等）
2. **niri-glass**（模糊 / 弹窗等玻璃拟态效果）
3. **鼠标特效**（本分支的主要亮点，见下文）

## 上游项目

- 官方 niri：[niri-wm/niri](https://github.com/niri-wm/niri)
- niri-glass：[zaroutt/Niri-glass](https://github.com/zaroutt/Niri-glass)
- Shorin-niri：[SHORiN-KiWATA/niri](https://github.com/SHORiN-KiWATA/niri)
- 鼠标特效参考实现：[DoomVoss/BASpark](https://github.com/DoomVoss/BASpark)

## 鼠标特效（光标特效）

参考 BASpark 风格实现的合成器内鼠标矢量粒子特效，全部用 GLES 直接在合成器内绘制，
不依赖任何外部程序。特效以**代码字符**的形式渲染（系统 `sans` 字体栅格化到 atlas，
配合稳定的伪随机种子，每次点击的字符、速度、半径、淡出时间都不同）。

### 特效预览

<img width="1920" height="1080" alt="左键爆裂" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E5%B7%A6%E9%94%AE.mp4" />

<img width="1920" height="1080" alt="右键爆裂" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E5%8F%B3%E9%94%AE.mp4" />

<img width="1920" height="1080" alt="中键爆裂" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E4%B8%AD%E9%94%AE.mp4" />

<img width="1920" height="1080" alt="滚轮向下（橙色顺时针）" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E8%BD%AE%E5%90%91%E4%B8%8B.mp4" />

<img width="1920" height="1080" alt="滚轮向上（绿色逆时针）" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E8%BD%AE%E5%90%91%E4%B8%8A.mp4" />

<img width="1920" height="1080" alt="滑轮粒子效果与速度关联" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E8%BD%AE%E7%B2%92%E5%AD%90%E6%95%88%E6%9E%9C%E5%92%8C%E6%BB%91%E8%BD%AE%E9%80%9F%E5%BA%A6%E7%9A%84%E5%85%B3%E8%81%94.mp4" />

<img width="1920" height="1080" alt="拖尾" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E5%8A%A8.mp4" />

### 特效预览

<img width="960" alt="左键爆裂" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E5%B7%A6%E9%94%AE.gif" />

<img width="960" alt="右键爆裂" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E5%8F%B3%E9%94%AE.gif" />

<img width="960" alt="中键爆裂" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E4%B8%AD%E9%94%AE.gif" />

<img width="960" alt="滚轮向下（橙色顺时针）" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E8%BD%AE%E5%90%91%E4%B8%8B.gif" />

<img width="960" alt="滚轮向上（绿色逆时针）" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E8%BD%AE%E5%90%91%E4%B8%8A.gif" />

<img width="960" alt="滑轮粒子效果与速度关联" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E8%BD%AE%E7%B2%92%E5%AD%90%E6%95%88%E6%9E%9C%E5%92%8C%E6%BB%91%E8%BD%AE%E9%80%9F%E5%BA%A6%E7%9A%84%E5%85%B3%E8%81%94.gif" />

<img width="960" alt="拖尾" src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E5%8A%A8.gif" />

### 点击爆裂

- 左键：**蓝色**（配置 `color`）
- 右键：**浅红色**
- 中键：**浅黄色**
- 每种按键的爆裂各自保存颜色快照，可同时在不同位置渲染，互不覆盖
- 圆盘 + 外圈环由代码字符组成，每字符独立的大小 / 速度 / 淡出窗口 → 逐个不规则消失
- 中键长按同样会出拖尾
- (硬编码请见谅)

### 滚轮代码圆环

- 滚轮**向下**：淡**橙**色圆环**顺时针**旋转
- 滚轮**向上**：淡**绿**色圆环**逆时针**旋转
- 旋转速度跟随滚轮滚动的快慢（按位移量比例注入）
- 转动时从环缘沿切向**甩出火花**，力度随机散布（角度抖动 + 径向分量）
- 圆环在可见期间始终**跟随光标**；停止滚动后惯性旋转片刻，再逐个字符淡出消失

### 拖尾

- 按住鼠标拖动产生密集代码字符拖尾（~2px 采样、640px 长）
- 拖尾火花淡出较慢，点击火花淡出较快

### 配置

在 niri 配置里添加 `cursor-effect { ... }` 块（均为可选，缺省取默认值）：

```kdl
cursor-effect {
    enabled true                    // 总开关（默认 true）
    scale 1.5                       // 特效缩放（默认 1.5）
    opacity 1.0                     // 整体不透明度（默认 1.0）
    color "45,175,255"              // 左键爆裂颜色，R,G,B 或 #rrggbb（默认 "45,175,255"）
    effect-speed 1.0                // 动画速度基准（默认 1.0）
    use-linked-animation-speed true // 拖尾/点击共用 effect-speed（默认 true）
    trail-speed 1.0                 // 拖尾速度（use-linked 时被忽略）
    click-speed 1.0                 // 点击速度（use-linked 时被忽略）
    trail-refresh-rate 40           // 拖尾采样频率 Hz（默认 40）
    enable-always-trail false       // 不按键也常显拖尾（默认 false）
    apply-curve-draw false          // 拖尾曲线平滑（默认 false）
    click-trigger "both"            // 哪些按键触发爆裂：left / right / both（默认 left）
    enable-middle-click-trigger true // 中键是否触发（默认 true）
    hide-in-fullscreen true         // 全屏时隐藏特效（默认 true）
    show-on-desktop true            // 桌面上显示特效（默认 true）
    touch-mode false                // 触摸屏模式（默认 false）
}
```

### 快捷键

特效开关动作：`toggle-cursor-effect`，例如：

```kdl
Mod+M hotkey-overlay-title="开关光标特效" repeat=false { toggle-cursor-effect; }
```

## 构建与使用

本仓库自带 flake：

```bash
nix develop . --command cargo build --bin niri
```

作为 niri 输入使用：

```kdl
inputs.niri-glass.url = "github:yigexuanmu/Niri-glass/beta";
```

> ⚠️ 再提醒一次：个人向分支，配置和行为以我自己的习惯为准，遇到问题请以官方
> [niri-wm/niri](https://github.com/niri-wm/niri) 为准。
