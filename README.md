<div align="center">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/logo-dark.svg">
    <img src="docs/brand/logo-light.svg" width="340" height="100" alt="VRCLT">
  </picture>
</h1>

<p><a href="https://vrclt.github.io/">Official site</a></p>

### Speak your language. Be heard in theirs.

Real-time voice translation and live subtitles for **VRChat** and **Discord** on Windows.

[![Latest release](https://img.shields.io/github/v/release/VRCLT/VRCLT?style=flat-square&label=release&color=2ea44f)](https://github.com/VRCLT/VRCLT/releases/latest) [![Downloads](https://img.shields.io/github/downloads/VRCLT/VRCLT/total?style=flat-square&color=1f6feb)](https://github.com/VRCLT/VRCLT/releases) [![Windows 10 | 11](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?style=flat-square)](https://github.com/VRCLT/VRCLT/wiki/Installation) [![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

**English** · [한국어](README.ko.md) · [日本語](README.ja.md) · [中文](README.zh.md)

<br>

<img src="docs/images/dashboard-en.png" width="780" alt="VRCLT Dashboard in the Simple view">

<br><br>

[**⬇ Download VRCLT**](https://github.com/VRCLT/VRCLT/releases/latest) &nbsp;·&nbsp; [Quick start](#quick-start) &nbsp;·&nbsp; [Documentation](#documentation)

</div>

## Why VRCLT

<table>
<tr>
<td width="50%" valign="top">

### 🎙️ Your voice, translated

Speak normally. VRCLT translates in real time and plays a natural translated voice into VRChat or Discord as your microphone, so others simply hear you in their language.

</td>
<td width="50%" valign="top">

### 💬 Live subtitles

Everyone else's speech appears as subtitles in SteamVR, in a desktop overlay, and in the app, with speaker labels when the engine separates speakers.

</td>
</tr>
<tr>
<td width="50%" valign="top">

### ⚡ One-click setup

The Setup check shows what is missing. **Auto setup** routes the audio, picks your microphone, and sets VRChat's microphone and OSC; **Install VB-Cable** runs the official installer for you.

</td>
<td width="50%" valign="top">

### 🥽 Built for VR

A wrist menu and a SteamVR Dashboard page keep the controls in reach, subtitles float in front of you, and VRCLT can start together with SteamVR.

</td>
</tr>
<tr>
<td width="50%" valign="top">

### 💭 VRChat chatbox

Your original line and its translation go to the chatbox over OSC. Text-only mode keeps your own voice and just adds the translated text.

</td>
<td width="50%" valign="top">

### 🌐 Four engines, 70+ languages

Google Gemini Live, Qwen3.8 LiveTranslate, OpenAI gpt-realtime-translate, and Soniox: choose by price, region, voice, and speaker separation.

</td>
</tr>
</table>

## Quick start

1. **Download** `vrclt-v<version>-windows-x64.exe` from [Releases](https://github.com/VRCLT/VRCLT/releases/latest) and run it. There is nothing to install.
2. **Add an API key.** Gemini is the default. [Get a key for any engine](https://github.com/VRCLT/VRCLT/wiki/API-Keys) (step-by-step with screenshots), then paste it into **Settings → Translation**.
3. **Let the Setup check do the rest.** Click **Install VB-Cable** if it is missing, close VRChat, then click **Auto setup**.
4. **Start VRChat and talk.** Toggle **Translate** and **Subtitles** on the Dashboard or from the wrist menu.

<p align="center"><img src="docs/images/setup-check-en.png" width="760" alt="Setup check with Auto setup"></p>

## In VR

<p align="center">
<img src="docs/images/vr-subtitles-en.png" width="620" alt="Subtitles in SteamVR"><br><br>
<img src="docs/images/vr-wrist-en.png" width="300" alt="Wrist menu">&nbsp;&nbsp;
<img src="docs/images/vr-dashboard-en.png" width="300" alt="SteamVR Dashboard page">
</p>

Subtitles float in front of you, the wrist menu switches translation, subtitles, and languages, and the SteamVR Dashboard page holds languages, audio devices, and panel layout.

## Documentation

Full guides live in the [wiki](https://github.com/VRCLT/VRCLT/wiki):

| | |
| --- | --- |
| [Installation](https://github.com/VRCLT/VRCLT/wiki/Installation) | Requirements, VB-Cable, first launch, and where files live |
| [API Keys](https://github.com/VRCLT/VRCLT/wiki/API-Keys) | Getting a Gemini, Qwen, OpenAI, or Soniox key, with screenshots |
| [Translation Engines](https://github.com/VRCLT/VRCLT/wiki/Translation-Engines) | How the four engines compare: languages, voices, regions |
| [Using VRCLT](https://github.com/VRCLT/VRCLT/wiki/Using-VRCLT) | All features, app modes, the Dashboard and Settings, audio routing |
| [VRChat and VR](https://github.com/VRCLT/VRCLT/wiki/VRChat-and-VR) | Chatbox, avatar OSC, SteamVR subtitles, wrist menu, SteamVR Dashboard |
| [Troubleshooting](https://github.com/VRCLT/VRCLT/wiki/Troubleshooting) | Common problems and how to fix them |
| [Configuration](https://github.com/VRCLT/VRCLT/wiki/Configuration) | Every `config.yaml` key |
| [Development](https://github.com/VRCLT/VRCLT/wiki/Development) | Running from source, tests, builds, and releases |

## Thanks To

- [Noto Sans CJK](https://github.com/notofonts/noto-cjk) and [Pretendard](https://github.com/orioncactus/pretendard) for multilingual UI font coverage.
- [PySide6](https://doc.qt.io/qtforpython-6/) for the native Windows interface.
- [OpenVR](https://github.com/ValveSoftware/openvr), GLFW, and PyOpenGL for SteamVR overlay rendering.
- [VB-Audio Virtual Cable](https://vb-audio.com/Cable/) for practical app-to-app audio routing.
