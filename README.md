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

[![Latest release](https://img.shields.io/github/v/release/VRCLT/VRCLT?style=flat-square&label=release&color=2ea44f)](https://github.com/VRCLT/VRCLT/releases/latest) [![Downloads](https://img.shields.io/github/downloads/VRCLT/VRCLT/total?style=flat-square&color=1f6feb)](https://github.com/VRCLT/VRCLT/releases) [![Windows 10 | 11](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?style=flat-square)](#requirements) [![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

**English** · [한국어](README.ko.md) · [日本語](README.ja.md) · [中文](README.zh.md)

<br>

<img src="docs/images/dashboard-en.png" width="780" alt="VRCLT Dashboard in the Simple view">

<br><br>

[**⬇ Download VRCLT**](https://github.com/VRCLT/VRCLT/releases/latest) &nbsp;·&nbsp; [Quick start](#quick-start) &nbsp;·&nbsp; [Documentation](#installation)

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
2. **Add an API key.** Gemini is the default: create a key at [Google AI Studio](https://aistudio.google.com/apikey) and paste it into **Settings → Gemini API key**. Other engines are described under [Translation Engines](#translation-engines).
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

<details>
<summary><b>All features</b></summary>

- Native Windows UI with Dashboard, Settings, and Logs/About tabs
- Tray menu for opening the app, opening settings, toggling translation/subtitles, and quitting
- Outbound translation: your microphone -> selected engine -> translated voice -> target app mic
- Inbound subtitles: target app audio -> selected engine -> translated subtitles
- Four translation engines: Google Gemini Live (default), Alibaba Qwen3.8 LiveTranslate, OpenAI gpt-realtime-translate, and Soniox
- VRChat support for OSC chatbox output, avatar OSC controls, SteamVR subtitles, and the wrist menu
- SteamVR dashboard settings panel and auto-start with SteamVR (Startup/Overlay Apps registration)
- VRC Text Only mode for sending translated chatbox text while passing your original voice through
- Discord mode with Discord process audio capture and VRChat-only features disabled
- Custom mode for capturing any other app, picked from the processes that are playing audio right now
- Low-latency raw mic passthrough with separate Gemini resampling for translation
- GitHub Releases update notification and safe config reset that keeps your API key, saved language lists, UI language, close action, and selected audio devices
- Downloadable Windows exe from [VRCLT Releases](https://github.com/VRCLT/VRCLT/releases)
- User settings stored in `%LOCALAPPDATA%\vrclt\config.yaml`

</details>

## Installation

### Requirements

- Windows 11 recommended
- An API key for your selected engine: Gemini, Qwen/DashScope, OpenAI, or Soniox
- [VB-Audio Virtual Cable](https://vb-audio.com/Cable/)
- SteamVR for VR overlays and wrist UI
- VRChat OSC enabled for chatbox/avatar-control features

### 1. Download vrclt

Download the latest Windows executable from [VRCLT Releases](https://github.com/VRCLT/VRCLT/releases).

Use the file named like:

```text
vrclt-v<version>-windows-x64.exe
```

The release executable stores settings in:

```text
%LOCALAPPDATA%\vrclt\config.yaml
```

The API key is stored as plain text in that file.

### 2. Install VB-Audio Virtual Cable

VB-Audio Virtual Cable is required when you want VRChat or Discord to hear the translated voice as a microphone.

**Easiest:** on the Dashboard's **Setup check** panel, click **Install VB-Cable**. vrclt downloads the official package from www.vb-cable.com, checks VB-Audio's digital signature, and opens VB-Audio's own setup: approve the Windows prompt, click **Install Driver**, then close the completion dialog. vrclt rereads the devices and continues with **Auto setup**. VB-CABLE is a donationware by VB-Audio, all participations are welcome.

To install it manually:

1. Download **VB-CABLE** from [VB-Audio Virtual Cable](https://vb-audio.com/Cable/).
2. Extract the downloaded ZIP file.
3. Right-click `VBCABLE_Setup_x64.exe` and choose **Run as administrator**.
4. Click **Install Driver**.
5. Restart Windows if `CABLE Input` / `CABLE Output` do not appear.

After installation, Windows will show two important devices:

| Device | What it means | Use it in |
| --- | --- | --- |
| `CABLE Input` | Playback/output side of the virtual cable | Select this as vrclt's translated voice output |
| `CABLE Output` | Recording/microphone side of the virtual cable | Select this as the microphone in VRChat or Discord |

Keep your real microphone selected in vrclt. Do not set VRChat or Discord to your real microphone if you want them to receive translated voice; set them to `CABLE Output`.

### 3. How to Get a Gemini API Key

1. Go to [Google AI Studio](https://aistudio.google.com/) and sign in with your Google account.
   - If you do not have a Google account, create one first.
2. Click the **Get API key** button in the left sidebar (or at the top of the page).
   - You can also navigate directly to [https://aistudio.google.com/apikey](https://aistudio.google.com/apikey).
3. Click **Create API key**.
4. Select a Google Cloud project to associate with the key.
   - If you have no existing projects, choose **Create API key in new project** and one will be created automatically.
5. Copy the generated key (it starts with `AIza...`).
   - Store it somewhere safe — it is only shown in full once.
6. Paste the key into the **Gemini API key** field in the vrclt Settings tab,
   or set it as `api_key` in `config.yaml`.

> **Note**: The Gemini API has a free tier with per-minute request limits that is sufficient for personal use.
> Do not share your API key. It is stored as plain text in `config.yaml`, so do not commit that file to a public repository.

### 3b. Alternative: Qwen API Key (for regions without Gemini access)

If Google services are not available in your region (for example mainland
China), vrclt can use **Alibaba Qwen3.8 LiveTranslate** instead of Gemini:

1. Create an Alibaba Cloud account and enable **Model Studio**:
   - Mainland China: [bailian.console.aliyun.com](https://bailian.console.aliyun.com/)
   - International (Singapore): [Alibaba Cloud Model Studio](https://www.alibabacloud.com/en/product/modelstudio)
2. Create an API key (DashScope key, starts with `sk-...`).
   **Keys are region-bound**: a mainland (Beijing) key only works with the
   `beijing` endpoint and an international key only with `intl`.
3. In vrclt Settings set **Translation engine** to `qwen`, paste the key into
   **Qwen API key (DashScope)**, and pick the matching **Qwen endpoint**
   (`beijing` for mainland China, `intl` otherwise). The key can also come
   from the `DASHSCOPE_API_KEY` environment variable.
4. **Qwen workspace ID — required for `intl`**: the international (Singapore)
   endpoint is only served through a workspace-scoped domain. On the Model
   Studio console homepage click the icon in the lower-left corner, then
   **Workspace Details**, and copy the ID (looks like `llm-7c72iiw36kd8****`)
   into the **Qwen workspace ID** field. The `beijing` endpoint can leave it
   empty (classic shared `dashscope` domain).
   See [Obtain the workspace ID](https://www.alibabacloud.com/help/en/model-studio/obtain-api-key-app-id-and-workspace-id).
5. The default model, `qwen3.8-livetranslate-flash-realtime`, detects the
   spoken language and separates speakers automatically. Only if you set
   **Qwen model** to `qwen3.5-livetranslate-flash-realtime`, also set **My
   spoken language** and **Others' spoken language** — Qwen 3.5 cannot
   auto-detect them (see [Translation Engines](#translation-engines)).

### 3c. Soniox API Key

1. Open the [Soniox Console](https://console.soniox.com), create an account, and sign in.
2. Open **My First Project** or your chosen project, then create a key under
   **API Keys**. See the [official setup guide](https://soniox.com/docs/stt/get-started).
   v1.0.0 uses US endpoints, so choose a **US project** if region selection is
   available; switching to other regional endpoints is not yet configurable.
   See [Soniox regions](https://soniox.com/docs/data-residency).
3. Check your API balance in [Billing overview](https://console.soniox.com/org/billing/overview/)
   and add funds if needed. Review [API pricing](https://soniox.com/pricing);
   any trial-credit availability and amount depend on the offer shown in your Console.
4. In vrclt, select `soniox` under **Settings → Translation engine** and paste
   the key into **Soniox API key**. Keep the initial defaults: recognition model
   `stt-rt-v5`, TTS model `tts-rt-v2`, and voice `Daniel`.
5. Choose the app mode, physical microphone, and translated voice output, then
   click **Save settings and restart**. Follow **First launch** below for
   `CABLE Input` / `CABLE Output` routing.
6. On the Dashboard, **Output language** is the language your speech is translated
   into; **Subtitle language** is the language you read others' speech in. For
   Korean-to-Japanese conversations, for example, choose Japanese output and
   Korean subtitles. Leave **My/Their spoken language** on automatic detection,
   or choose Korean/Japanese as recognition hints.

To use `SONIOX_API_KEY`, leave the app's Soniox API key field empty and fully
quit/relaunch the app after setting the environment variable. For manual
configuration, use the following `config.yaml` values. No file editing is
needed when you use the GUI:

```yaml
provider: soniox
soniox:
  api_key: ""  # empty uses SONIOX_API_KEY
  model: stt-rt-v5
  tts_model: tts-rt-v2
  voice: Daniel
  keep_speaker_context: true
  speaker_context_idle_sec: 60.0
```

Soniox streams speech recognition and translation for both directions. When
translated voice is enabled, it also uses Soniox TTS. `voice` accepts a stock
voice name or an existing custom/cloned voice ID; it does not automatically
clone your microphone voice. Text-only pipelines do not open a TTS session.
**My/Their spoken language** can stay on Auto or provide an optional language
hint.

Speaker diarization is enabled: source text and translations stay associated
with each speaker, and subtitles display speaker numbers. These are session-local
labels, not VRChat account identities; a reconnect starts a new recognition
session. Only committed tokens are published so provisional speaker assignments
do not repeatedly rewrite chatbox text or speech. The selected TTS voice applies
to all speakers. See [Soniox speaker diarization](https://soniox.com/docs/stt/concepts/speaker-diarization)
and [available voices](https://soniox.com/docs/tts/concepts/voices).

**Keep speaker context** is on by default (`soniox.keep_speaker_context:
true`). It keeps the recognition session through short pauses and disconnects
after **60 seconds of silence** (`soniox.speaker_context_idle_sec: 60.0`).
Adjust this timeout in Settings (5–600 seconds). A longer timeout avoids more
reconnections but keeps a billable connection open longer. When Soniox is
selected, the Dashboard shows the checkbox and effective silence timeout;
the checkbox uses the same saved value as Settings. Existing saved choices
are preserved. Speaker diarization remains enabled with either choice.
Soniox bills the full connected stream, including silence and keepalive
periods, even in text-only mode.
See [Soniox keepalive billing](https://soniox.com/docs/stt/rt/connection-keepalive).
With this setting off, the connection instead uses
`audio.mic_idle_disconnect_sec` (15 seconds by default). Stopping/disabling the pipeline or
ending the captured process also closes its connection. Reconnecting starts a
new speaker context. Keeping a session open does not guarantee that one person
always receives the same speaker number; different IDs returned by Soniox are
kept separate.

This integration has local mock-server coverage; paid live Soniox calls have
not been verified.

**First check:** enable **Subtitles** while someone speaks in the captured app.
Look for translated subtitles and speaker numbers/colors in the Dashboard's
live preview; take turns with multiple speakers to check separation. Enable
**Translation** and speak a short sentence to check your outgoing translation.

- Authentication error (`401`): check that Soniox is selected and the key is valid for a US project.
- Balance/budget error (`402`): check the Console balance and project/organization
  monthly limits. See the [error reference](https://soniox.com/docs/api-reference/errors).
- No subtitles or audio: check the captured app/process, translation/subtitle
  switches, microphone, and output device, then inspect **Logs/About**. If text
  works but translated voice does not, also check **Text only** and voice output.

### 4. First Launch Setup

1. Run `vrclt-v<version>-windows-x64.exe`.
2. Open the Settings tab and set up the translation engine you use:

   **Using Gemini (default)**
   1. Paste your Gemini API key (step 3) into **Gemini API key**.
      That is all engine setup — Gemini detects spoken languages automatically.

   **Using Qwen**
   1. Set **Translation engine** to `qwen`.
   2. Paste your DashScope key (step 3b) into **Qwen API key (DashScope)** and
      pick the **Qwen endpoint** matching the key's region (`beijing` for
      mainland China, `intl` otherwise).
   3. `intl` only: fill in the **Qwen workspace ID** (step 3b).
   4. That is all for the default `qwen3.8-livetranslate-flash-realtime`
      model — it detects spoken languages automatically. With
      `qwen3.5-livetranslate-flash-realtime`, also set **My spoken language**
      (the language you speak) and **Others' spoken language** (the language
      you expect to hear in-game); Qwen 3.5 cannot auto-detect them, and
      empty is treated as English. For Qwen 3.5 both can be changed later on
      the Dashboard tab or the SteamVR dashboard panel.

   **Using Soniox**: select `soniox` and enter your key as described in step 3c.
   **Using OpenAI**: select `openai` and enter your OpenAI API key.

3. Choose the app mode: `vrchat`, `discord`, or `custom` (then pick the app to capture in Settings).
4. Select your physical microphone as **Mic input**.
5. Select `CABLE Input` as **Voice output** / translated voice output.
6. In VRChat or Discord, set the microphone input to **CABLE Output (VB-Audio Virtual Cable)**.
7. Save settings. The runtime restarts automatically.

**Auto setup (recommended):** the Dashboard opens in the **Simple** view with a **Setup check** panel that lists what is ready: API key, VB-Cable, translated voice output, your microphone, and VRChat's microphone and OSC. Close VRChat, then press **Auto setup**: it sends the translated voice to `CABLE Input`, picks your real microphone (the one VRChat used before, otherwise the system default), sets VRChat's microphone to `CABLE Output` for VR and desktop, and turns on VRChat's OSC. **Find my mic** picks the microphone that reacts most while you speak. Steps 3–6 above are what Auto setup does; anything it cannot change shows instructions in the panel. Switch to **Full** (top right) for every dashboard control.

## Troubleshooting

- PC sound stopped after installing VB-Cable: Windows made `CABLE Input` the default playback device. Set your speakers or headphones as the default again (**Setup check → Open sound settings**).
- Auto setup did not change VRChat's microphone or OSC: close VRChat first. VRChat overwrites its settings when it exits, so vrclt only writes them while VRChat is closed.
- No translated voice in the target app: confirm `outbound.tts_device` is `CABLE Input` and the target app microphone is `CABLE Output`.
- No inbound subtitles: confirm the target process name matches the running app, for example `VRChat.exe` or `Discord.exe`.
- Runtime says API key is required: enter the selected engine's key in Settings or set `GEMINI_API_KEY`, `DASHSCOPE_API_KEY`, `OPENAI_API_KEY`, or `SONIOX_API_KEY`.
- Qwen key rejected or connection fails immediately: check that `qwen.endpoint` matches the key's region — `beijing` keys and `intl` keys are not interchangeable — and that the workspace ID is set when using `intl`.
- Qwen error `Voice '...' is not supported`: the literal voice `default` only works while voice cloning is enabled. Keep `qwen.voice_clone` on, or leave **Qwen voice ID** empty for the model's default voice.
- Qwen 3.5 translates from the wrong language: set **My spoken language** / **Others' spoken language** (Settings, Dashboard, or the SteamVR panel). Qwen 3.5 cannot auto-detect; empty is treated as English. Qwen 3.8 (the default model) detects it automatically.
- VR overlays do not appear: confirm SteamVR is running and `overlay.enabled` / `wrist_ui.enabled` are enabled.
- Passthrough or subtitles feel late: start from the defaults in this README, then lower `audio.turn_end_silence_sec`, `audio.inbound_turn_end_silence_sec`, or `audio.subtitle_finalize_silence_sec` carefully if your connection is stable.
- Need a clean config: use **Reset defaults** in Settings. It resets current settings while preserving the API key, output language list, subtitle language list, UI language, window close action, and selected audio devices. After an app update, vrclt also asks once whether to do this reset.

## Translation Engines

vrclt supports four realtime translation engines, selected by the **Translation
engine** setting (`provider` in `config.yaml`). The choice applies to both
directions: your voice and the inbound subtitles.

| | Gemini Live (default) | Qwen3.8 LiveTranslate | gpt-realtime-translate | Soniox |
| --- | --- | --- | --- | --- |
| Provider / key | Google AI Studio (`GEMINI_API_KEY`) | Alibaba Cloud Model Studio / DashScope (`DASHSCOPE_API_KEY`) | OpenAI (`OPENAI_API_KEY`) | Soniox (`SONIOX_API_KEY`) |
| Required setup | API key only | Engine to `qwen`, API key + endpoint (workspace ID on `intl`); Qwen 3.5 also needs **My/Their spoken language** | Engine to `openai`, API key only | Engine to `soniox`, API key |
| Works in mainland China | No | Yes (`beijing` endpoint) | No | Not verified |
| Spoken-language detection | Automatic | Automatic (Qwen 3.5: **manual** — set "My/Their spoken language") | Automatic (the spoken-language settings are ignored) | Automatic; optional spoken-language hints |
| Languages | 70+ BCP-47 targets, incl. `zh-Hans`/`zh-Hant` | 29 with voice + 31 more text-only; plain `zh` only (no Simplified/Traditional split); Cantonese (`yue`) is text-only | 70+ inputs, but only **13 targets** (`en es pt fr ja ru zh de ko hi id vi it`); plain `zh` only; one target per session | 60 in vrclt; plain `zh` only; no Cantonese (`yue`) |
| Translated voice | Replicates the speaker's voice | Replicates the speaker via server-side voice cloning (`qwen.voice_clone`, default `once`); or a fixed voice with cloning off | Adapts to the speaker automatically; no voice option | Separate TTS; stock/custom voice, default `Daniel`; no automatic cloning |
| Barge-in (voice interrupts) | Yes | No — overlapping speech can queue audio | No — overlapping speech can queue audio | No — overlapping speech can queue audio |

Qwen notes:

- `qwen.model` defaults to `qwen3.8-livetranslate-flash-realtime`. It detects
  the spoken language automatically and labels subtitles by speaker, like
  Soniox. Saved configs keep the model they already have; change **Qwen
  model** to switch.
- `qwen3.5-livetranslate-flash-realtime` still works, but **requires the
  spoken language**: set it in Settings, on the Dashboard tab, or on the
  SteamVR Dashboard's **Live** page. Empty is treated as English. Qwen 3.5
  has no speaker labels.
- `zh-Hans`/`zh-Hant` targets are both sent to Qwen as `zh`.
- If the chosen target has no Qwen voice support, vrclt automatically runs the
  session text-only (chatbox/subtitles keep working).
- Endpoints: `intl` (Singapore) or `beijing`. `intl` **requires** the Model
  Studio workspace ID; `beijing` works without one (see step 3b above).
- Voice cloning: `qwen.voice_clone: once` (default) clones the speaker's voice
  at session start — right for your own microphone and keeps audio latency
  low. `always` re-clones per response (multi-speaker audio, but synthesis
  starts noticeably later); `off` uses the model's default voice or a
  pre-cloned voice ID set in `qwen.voice` (`qwen-translate-vc-...`).

## App Modes

| Mode | Use for | Behavior |
| --- | --- | --- |
| `vrchat` | VRChat | Captures `VRChat.exe`, enables OSC chatbox, avatar OSC control, SteamVR subtitles, and wrist UI |
| `discord` | Discord | Captures the root `Discord.exe` process tree, disables VRChat OSC/SteamVR features, keeps the native PC UI and desktop subtitles active |
| `custom` | Any other app | Captures the process you pick in Settings, keeps SteamVR subtitles and the wrist menu, disables the VRChat OSC features |

Choose a mode in Settings or pass it for one launch:

```powershell
.\vrclt.exe run --app vrchat
.\vrclt.exe run --app discord
.\vrclt.exe run --app custom
```

For VRChat text-only behavior, enable **Text only** in the Dashboard or
Settings. Your original microphone passes through to VRChat while the selected engine sends
translated text to the OSC chatbox without translated voice output.

For Discord Canary or PTB, change the Discord process name in Settings or in
`app.profiles.discord.process`.

To caption any other app - a browser, a media player, another game - switch to
**Custom** mode and set **Custom capture process** in Settings. Opening that
dropdown lists the processes that hold a Windows audio session, with the ones
playing sound right now marked and sorted first, so you never have to guess an
exe name. The same picker backs **Capture process**, which is whatever is being
captured at this moment.

## Native UI

Dashboard:

- **Simple / Full** view switch (top right). New installs start in Simple: the setup check (one line once everything is ready), the app choice (VRChat/Discord/custom), translation/subtitle toggles, their languages, your microphone with **Find my mic**, a mic level bar, and live subtitles. Full adds the text-only switch, devices, the engine button, language-list editing, spoken languages, volume, and display/app options. Microphone lists never offer `CABLE Output`, which would only feed the translated voice back. Configurations saved before v1.0.0 keep the full view.
- **Setup check** panel with **Auto setup**, **Find my mic**, and **Install VB-Cable**; it folds to one line when everything is ready
- A compact status header shows the selected engine with a shortcut to its settings.
- Main translation, subtitle, language, and audio controls stay together. Language-list editing and less-used display/app controls open in expandable sections.
- Runtime status and connection state
- VRChat/Discord/Custom mode toggle and VRChat text-only toggle
- Translation ON/OFF
- Subtitles ON/OFF
- Global PC hotkeys for translation/subtitle toggles
- Output language and subtitle language, with searchable add controls for 70+
  Gemini Live Translation languages
- Microphone input and translated voice output device selectors, with an
  output test-tone button; Refresh devices restarts the runtime and picks up
  hot-plugged devices
- Translated-voice volume slider and a live mic level meter with the
  detection-threshold marker
- My/Their spoken language pickers: required for Qwen 3.5, optional recognition hints for Soniox; Gemini, OpenAI, and Qwen 3.8 auto-detect
- PC subtitle position, box size, and font size controls
- Live subtitle preview

Settings:

- Category navigation and search; only the selected engine's provider-specific settings are shown
- Translation engine (Gemini / Qwen / OpenAI / Soniox), API keys, models, and the Qwen
  endpoint/workspace
- App mode and target processes
- Microphone, translated voice output, monitor output, and inbound audio device
- Default target languages and saved language lists
- Global PC hotkey settings
- Audio thresholds and VAD settings
- Reset defaults button that keeps the API key, output language list, subtitle language list, UI language, window close action, and selected audio devices
- OSC, chatbox, SteamVR overlay, and wrist UI options
- UI language and UI mode

Logs/About:

- Current config path
- Current log file path
- Recent log tail
- Log bursts update the view once per poll; catch-up reads are limited to 256 KiB, history to 2,000 lines, and incomplete lines to 64 KiB

Reconnect delays wait for the stop event directly, avoiding periodic polling and
responding immediately when the runtime stops.

Closing the window hides it to the tray. Use the tray `Quit` action to stop the
runtime and exit.

## Audio Routing

Outbound translation:

```text
microphone -> selected engine -> translated voice -> CABLE Input
                                     target app mic <- CABLE Output
```

Inbound subtitles:

```text
target app process audio -> ProcTap -> selected engine -> subtitles
```

When translation is OFF, the microphone bypasses Gemini and is sent directly to
`CABLE Input`. In VRChat **Text only**, the original microphone is always passed
through; the translation toggle controls Gemini text translation and chatbox
output.

Raw passthrough uses the captured 48 kHz microphone stream directly, while the
Gemini translation stream is resampled separately. This keeps passthrough from
waiting on the translation session and avoids unnecessary quality loss.

## VRChat Features

VRChat mode can use:

- OSC chatbox output for translated text
- Avatar OSC parameters such as `VRCLT_Enabled` and `VRCLT_Lang`
- SteamVR subtitle overlay for inbound subtitles
- A compact SteamVR wrist menu with **Live** and **Settings** pages. **Live** contains translation, subtitles, output/subtitle languages, connection status, separate **Voice / Text** buttons, and runtime restart. **Settings** groups UI language, subtitle size, panel movement/reset, and Soniox speaker-context retention. Gaze/grip interactions remain available
- The wrist menu defaults to **14 cm** wide with a shorter panel shape. Existing saved widths are preserved; adjust the wrist width in Settings to resize it. Widths below 16 cm now apply correctly
- SteamVR Dashboard with **Live**, **Audio**, and **Layout & app** pages (open the SteamVR menu and pick the vrclt icon). **Live** groups translation, subtitles, languages, and connection/error status; **Audio** contains microphone/voice-output selection and translated-voice volume; **Layout & app** groups panel positioning and app controls. Device changes apply shortly after the last click with a runtime restart
- The SteamVR Dashboard offers spoken-language controls for Qwen 3.5 and optional recognition hints for Soniox; the wrist menu selects translation output and subtitle languages. With Soniox selected, both VR panels show **Keep speaker context** and the effective silence timeout; the switch shares the saved setting with the PC UI. Speaker diarization stays enabled when the switch is off
- Auto-start with SteamVR: the release exe registers itself in SteamVR Settings > Startup/Overlay Apps; toggle auto-launch there or in vrclt Settings
- After updating to a new release, run the new exe once so auto-start points at the new file (the registration itself survives updates; only the recorded exe path needs that first run to refresh)
- Visible VR subtitle edit laser/cursor with corner resize handles

With `ui.mode: auto`, SteamVR features activate when SteamVR is running. Use
`ui.mode: vr` to force VR overlays or `ui.mode: desktop` to keep them disabled.

## Files And Paths

| Item | Path |
| --- | --- |
| Config | `%LOCALAPPDATA%\vrclt\config.yaml` |
| Logs | `%LOCALAPPDATA%\vrclt\logs\vrclt.log` |

Set the `VRCLT_CONFIG` environment variable to use a different config file.
Every `config.yaml` key is described in the wiki's
[Configuration reference](https://github.com/VRCLT/VRCLT/wiki/Configuration).

## Development

Running from source, tests, building the exe, and the release flow are
documented in the [wiki](https://github.com/VRCLT/VRCLT/wiki/Development). Changes reach `main` through pull requests,
and merging a new version publishes its release automatically.

## Thanks To

- [Noto Sans CJK](https://github.com/notofonts/noto-cjk) and [Pretendard](https://github.com/orioncactus/pretendard) for multilingual UI font coverage.
- [PySide6](https://doc.qt.io/qtforpython-6/) for the native Windows interface.
- [OpenVR](https://github.com/ValveSoftware/openvr), GLFW, and PyOpenGL for SteamVR overlay rendering.
- [VB-Audio Virtual Cable](https://vb-audio.com/Cable/) for practical app-to-app audio routing.
