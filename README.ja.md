<div align="center">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/brand/logo-dark.svg">
    <img src="docs/brand/logo-light.svg" width="340" height="100" alt="VRCLT">
  </picture>
</h1>

<p><a href="https://vrclt.github.io/ja/">公式サイト</a></p>

### 自分の言葉で話せば、相手の言葉で届く。

Windows 向け **VRChat** / **Discord** リアルタイム音声翻訳とリアルタイム字幕。

[![Latest release](https://img.shields.io/github/v/release/VRCLT/VRCLT?style=flat-square&label=release&color=2ea44f)](https://github.com/VRCLT/VRCLT/releases/latest) [![Downloads](https://img.shields.io/github/downloads/VRCLT/VRCLT/total?style=flat-square&color=1f6feb)](https://github.com/VRCLT/VRCLT/releases) [![Windows 10 | 11](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?style=flat-square)](https://github.com/VRCLT/VRCLT/wiki/%E3%82%A4%E3%83%B3%E3%82%B9%E3%83%88%E3%83%BC%E3%83%AB) [![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

[English](README.md) · [한국어](README.ko.md) · **日本語** · [中文](README.zh.md)

<br>

<img src="docs/images/dashboard-ja.png" width="780" alt="シンプル表示の VRCLT ダッシュボード">

<br><br>

[**⬇ VRCLT をダウンロード**](https://github.com/VRCLT/VRCLT/releases/latest) &nbsp;·&nbsp; [クイックスタート](#クイックスタート) &nbsp;·&nbsp; [詳しいドキュメント](#ドキュメント)

</div>

## VRCLT の特長

<table>
<tr>
<td width="50%" valign="top">

### 🎙️ あなたの声をそのまま翻訳

いつも通り話すだけ。VRCLT がリアルタイムで翻訳し、自然な翻訳音声を VRChat や Discord のマイクとして届けます。相手は自分の言葉であなたの声を聞けます。

</td>
<td width="50%" valign="top">

### 💬 リアルタイム字幕

相手の発話が SteamVR、デスクトップオーバーレイ、アプリ画面に字幕で表示されます。話者を区別できるエンジンでは話者番号も表示します。

</td>
</tr>
<tr>
<td width="50%" valign="top">

### ⚡ ワンクリック設定

設定チェックが足りないものを教えてくれます。**自動設定** は音声の経路、マイク選択、VRChat のマイクと OSC まで整え、**VB-Cableを自動インストール** は公式インストーラーを代わりに起動します。

</td>
<td width="50%" valign="top">

### 🥽 VR のためのアプリ

手首メニューと SteamVR ダッシュボードのページで VR の中から操作でき、字幕は目の前に浮かび、SteamVR と一緒に自動起動できます。

</td>
</tr>
<tr>
<td width="50%" valign="top">

### 💭 VRChat チャットボックス

原文と翻訳を OSC でチャットボックスに送ります。テキストのみモードでは自分の声はそのままに、翻訳テキストだけを加えます。

</td>
<td width="50%" valign="top">

### 🌐 4 つのエンジン、70 以上の言語

Google Gemini Live、Qwen3.8 LiveTranslate、OpenAI gpt-realtime-translate、Soniox から、料金・地域・音声・話者分離に合わせて選べます。

</td>
</tr>
</table>

## クイックスタート

1. [Releases](https://github.com/VRCLT/VRCLT/releases/latest) から `vrclt-v<version>-windows-x64.exe` を**ダウンロードして実行**します。インストールは不要です。
2. **API キーを入力します。** 既定のエンジンは Gemini です。[エンジン別のキー取得手順](https://github.com/VRCLT/VRCLT/wiki/API%E3%82%AD%E3%83%BC%E3%81%AE%E5%8F%96%E5%BE%97)（画面キャプチャ付き）でキーを取得し、**設定 → 翻訳・言語** に貼り付けます。
3. **残りは設定チェックにお任せ。** VB-Cable がなければ **VB-Cableを自動インストール** を押し、VRChat を終了してから **自動設定** を押します。
4. **VRChat を起動して話すだけ。** 翻訳と字幕はダッシュボードか手首メニューで切り替えます。

<p align="center"><img src="docs/images/setup-check-ja.png" width="760" alt="自動設定ボタンのある設定チェック"></p>

## VR で

<p align="center">
<img src="docs/images/vr-subtitles-ja.png" width="620" alt="SteamVR の字幕"><br><br>
<img src="docs/images/vr-wrist-ja.png" width="300" alt="手首メニュー">&nbsp;&nbsp;
<img src="docs/images/vr-dashboard-ja.png" width="300" alt="SteamVR ダッシュボードのページ">
</p>

字幕は目の前に浮かび、手首メニューで翻訳・字幕・言語を切り替え、SteamVR ダッシュボードのページで言語、オーディオデバイス、パネル配置を扱います。

## ドキュメント

詳しい説明は [Wiki](https://github.com/VRCLT/VRCLT/wiki) にあります。

| | |
| --- | --- |
| [インストール](https://github.com/VRCLT/VRCLT/wiki/%E3%82%A4%E3%83%B3%E3%82%B9%E3%83%88%E3%83%BC%E3%83%AB) | 要件、VB-Cable、初回起動、ファイルの場所 |
| [APIキーの取得](https://github.com/VRCLT/VRCLT/wiki/API%E3%82%AD%E3%83%BC%E3%81%AE%E5%8F%96%E5%BE%97) | Gemini・Qwen・OpenAI・Soniox のキー取得手順（画面キャプチャ付き） |
| [翻訳エンジン](https://github.com/VRCLT/VRCLT/wiki/%E7%BF%BB%E8%A8%B3%E3%82%A8%E3%83%B3%E3%82%B8%E3%83%B3) | 4 つのエンジンの比較: 言語、音声、地域 |
| [使い方](https://github.com/VRCLT/VRCLT/wiki/%E4%BD%BF%E3%81%84%E6%96%B9) | すべての機能、アプリモード、ダッシュボードと設定、音声の流れ |
| [VRChat と VR](https://github.com/VRCLT/VRCLT/wiki/VRChat-%E3%81%A8-VR) | チャットボックス、アバター OSC、SteamVR 字幕、手首メニュー、SteamVR ダッシュボード |
| [トラブルシューティング](https://github.com/VRCLT/VRCLT/wiki/%E3%83%88%E3%83%A9%E3%83%96%E3%83%AB%E3%82%B7%E3%83%A5%E3%83%BC%E3%83%86%E3%82%A3%E3%83%B3%E3%82%B0) | よくある問題と解決方法 |
| [設定リファレンス](https://github.com/VRCLT/VRCLT/wiki/%E8%A8%AD%E5%AE%9A%E3%83%AA%E3%83%95%E3%82%A1%E3%83%AC%E3%83%B3%E3%82%B9) | `config.yaml` のすべてのキー |
| [開発ガイド](https://github.com/VRCLT/VRCLT/wiki/%E9%96%8B%E7%99%BA%E3%82%AC%E3%82%A4%E3%83%89) | ソースからの実行、テスト、ビルド、リリース |

## 謝辞

- [Noto Sans CJK](https://github.com/notofonts/noto-cjk) と [Pretendard](https://github.com/orioncactus/pretendard): 多言語 UI フォントカバレッジ。
- [PySide6](https://doc.qt.io/qtforpython-6/): Windows ネイティブ UI。
- [OpenVR](https://github.com/ValveSoftware/openvr)、GLFW、PyOpenGL: SteamVR オーバーレイレンダリング。
- [VB-Audio Virtual Cable](https://vb-audio.com/Cable/): アプリ間オーディオルーティング。
