# 渲染测试

## A: video 标签在 details 内，URL 百分号编码
<details>
<summary>展开</summary>
<video src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E5%8A%A8.mp4" controls width="320"></video>
</details>

## B: video 标签直接内嵌，URL 百分号编码
<video src="https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E5%8A%A8.mp4" controls width="320"></video>

## C: 图片语法 ![]() 指向 mp4（Miyu 式写法）
![](https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/%E6%BB%91%E5%8A%A8.mp4)

## D: video 标签用 github.com/.../raw/... 形式
<video src="https://github.com/yigexuanmu/images/raw/main/Niri-beta/%E6%BB%91%E5%8A%A8.mp4" controls width="320"></video>

## E: 对照组 — Miyu 式图片（ASCII 路径）
![](https://raw.githubusercontent.com/SHORiN-KiWATA/Miyu/main/pics/webui.png)

## F: 对照组 — 图片语法指向中文名 png（先传一个测试 png 到 images/Niri-beta）
![](https://raw.githubusercontent.com/yigexuanmu/images/main/Niri-beta/test.png)
