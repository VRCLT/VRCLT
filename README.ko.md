<div align="center">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/logo-dark.svg">
    <img src="docs/brand/logo-light.svg" width="340" height="100" alt="VRCLT">
  </picture>
</h1>

<p><a href="https://vrclt.github.io/ko/">공식 홈페이지</a></p>

### 내 언어로 말하면, 상대의 언어로 들립니다.

Windows용 **VRChat** · **Discord** 실시간 음성 번역과 실시간 자막.

[![Latest release](https://img.shields.io/github/v/release/VRCLT/VRCLT?style=flat-square&label=release&color=2ea44f)](https://github.com/VRCLT/VRCLT/releases/latest) [![Downloads](https://img.shields.io/github/downloads/VRCLT/VRCLT/total?style=flat-square&color=1f6feb)](https://github.com/VRCLT/VRCLT/releases) [![Windows 10 | 11](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?style=flat-square)](https://github.com/VRCLT/VRCLT/wiki/%EC%84%A4%EC%B9%98) [![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

[English](README.md) · **한국어** · [日本語](README.ja.md) · [中文](README.zh.md)

<br>

<img src="docs/images/dashboard-ko.png" width="780" alt="간단히 모드의 VRCLT 대시보드">

<br><br>

[**⬇ VRCLT 내려받기**](https://github.com/VRCLT/VRCLT/releases/latest) &nbsp;·&nbsp; [빠른 시작](#빠른-시작) &nbsp;·&nbsp; [자세한 문서](#문서)

</div>

## VRCLT를 쓰는 이유

<table>
<tr>
<td width="50%" valign="top">

### 🎙️ 내 목소리 그대로 번역

평소처럼 말하면 VRCLT가 실시간으로 번역해 자연스러운 번역 음성을 VRChat·Discord 마이크로 보냅니다. 상대는 그냥 자기 언어로 내 말을 듣습니다.

</td>
<td width="50%" valign="top">

### 💬 실시간 자막

다른 사람의 말이 SteamVR, 데스크톱 오버레이, 앱 화면에 자막으로 뜹니다. 화자를 구분하는 엔진에서는 화자 번호도 함께 표시됩니다.

</td>
</tr>
<tr>
<td width="50%" valign="top">

### ⚡ 원클릭 설정

설정 점검이 빠진 것을 알려 줍니다. **자동 설정**은 오디오 연결, 마이크 선택, VRChat 마이크·OSC 설정까지 맞추고, **VB-Cable 자동 설치**는 공식 설치 프로그램을 대신 실행합니다.

</td>
<td width="50%" valign="top">

### 🥽 VR을 위해 만든 앱

손목 메뉴와 SteamVR 대시보드 페이지로 VR 안에서 바로 조작하고, 자막은 눈앞에 떠 있으며, SteamVR과 함께 자동으로 시작할 수 있습니다.

</td>
</tr>
<tr>
<td width="50%" valign="top">

### 💭 VRChat 챗박스

원문과 번역문을 OSC로 챗박스에 보냅니다. 텍스트 전용 모드는 내 목소리는 그대로 두고 번역 텍스트만 더합니다.

</td>
<td width="50%" valign="top">

### 🌐 엔진 4종, 70개 이상 언어

Google Gemini Live, Qwen3.8 LiveTranslate, OpenAI gpt-realtime-translate, Soniox 중에서 가격·지역·음성·화자 분리에 맞게 고를 수 있습니다.

</td>
</tr>
</table>

## 빠른 시작

1. [Releases](https://github.com/VRCLT/VRCLT/releases/latest)에서 `vrclt-v<version>-windows-x64.exe`를 **내려받아 실행**합니다. 설치 과정은 없습니다.
2. **API 키를 입력합니다.** 기본 엔진은 Gemini입니다. [엔진별 키 발급 방법](https://github.com/VRCLT/VRCLT/wiki/API-%ED%82%A4-%EB%B0%9C%EA%B8%89)(화면 캡처 포함)을 따라 키를 받은 뒤 **설정 → 번역·언어**에 붙여 넣으세요.
3. **나머지는 설정 점검에 맡깁니다.** VB-Cable이 없으면 **VB-Cable 자동 설치**를 누르고, VRChat을 끈 뒤 **자동 설정**을 누르세요.
4. **VRChat을 켜고 말하면 됩니다.** 번역·자막은 대시보드나 손목 메뉴에서 켜고 끕니다.

<p align="center"><img src="docs/images/setup-check-ko.png" width="760" alt="자동 설정이 있는 설정 점검"></p>

## VR에서

<p align="center">
<img src="docs/images/vr-subtitles-ko.png" width="620" alt="SteamVR 자막"><br><br>
<img src="docs/images/vr-wrist-ko.png" width="300" alt="손목 메뉴">&nbsp;&nbsp;
<img src="docs/images/vr-dashboard-ko.png" width="300" alt="SteamVR 대시보드 페이지">
</p>

자막은 눈앞에 떠 있고, 손목 메뉴로 번역·자막·언어를 바꾸며, SteamVR 대시보드 페이지에서 언어·오디오 장치·패널 배치를 다룹니다.

## 문서

자세한 안내는 [위키](https://github.com/VRCLT/VRCLT/wiki)에 있습니다.

| | |
| --- | --- |
| [설치](https://github.com/VRCLT/VRCLT/wiki/%EC%84%A4%EC%B9%98) | 요구사항, VB-Cable, 첫 실행, 파일 위치 |
| [API 키 발급](https://github.com/VRCLT/VRCLT/wiki/API-%ED%82%A4-%EB%B0%9C%EA%B8%89) | Gemini·Qwen·OpenAI·Soniox 키 발급 방법 (화면 캡처 포함) |
| [번역 엔진](https://github.com/VRCLT/VRCLT/wiki/%EB%B2%88%EC%97%AD-%EC%97%94%EC%A7%84) | 엔진 4종 비교: 언어, 음성, 지역 |
| [사용법](https://github.com/VRCLT/VRCLT/wiki/%EC%82%AC%EC%9A%A9%EB%B2%95) | 전체 기능, 앱 모드, 대시보드와 설정, 오디오 흐름 |
| [VRChat과 VR](https://github.com/VRCLT/VRCLT/wiki/VRChat%EA%B3%BC-VR) | 챗박스, 아바타 OSC, SteamVR 자막, 손목 메뉴, SteamVR 대시보드 |
| [문제 해결](https://github.com/VRCLT/VRCLT/wiki/%EB%AC%B8%EC%A0%9C-%ED%95%B4%EA%B2%B0) | 자주 생기는 문제와 해결 방법 |
| [설정 레퍼런스](https://github.com/VRCLT/VRCLT/wiki/%EC%84%A4%EC%A0%95-%EB%A0%88%ED%8D%BC%EB%9F%B0%EC%8A%A4) | `config.yaml`의 모든 키 |
| [개발 가이드](https://github.com/VRCLT/VRCLT/wiki/%EA%B0%9C%EB%B0%9C-%EA%B0%80%EC%9D%B4%EB%93%9C) | 소스 실행, 테스트, 빌드, 릴리스 |

## 감사

- [Noto Sans CJK](https://github.com/notofonts/noto-cjk)와 [Pretendard](https://github.com/orioncactus/pretendard): 다국어 UI 폰트 커버리지.
- [PySide6](https://doc.qt.io/qtforpython-6/): Windows 자체 UI.
- [OpenVR](https://github.com/ValveSoftware/openvr), GLFW, PyOpenGL: SteamVR 오버레이 렌더링.
- [VB-Audio Virtual Cable](https://vb-audio.com/Cable/): 앱 간 오디오 라우팅.
