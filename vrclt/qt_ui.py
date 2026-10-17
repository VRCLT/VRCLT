"""PySide6 native UI for vrclt."""
from __future__ import annotations

import copy
import logging
import os
import threading
import time
from pathlib import Path

from PySide6 import QtCore, QtGui, QtWidgets

from . import __version__
from . import config as config_mod
from . import platform_support, setup_check
from . import i18n
from .app_controller import resolve_ui_mode
from .config import get_path as _get_path, set_path as _set_path
from .desktop_overlay import DesktopSubtitleOverlay
from .languages import language_code_from_text, language_label
from .hotkeys import HotkeyRegistration, WindowsGlobalHotkeys
from .resources import bundled_font, icon_path, resolve_font_path
from .ui import theme
from .ui.log_view import LogPanel
from .ui.settings_form import SettingsForm, SettingsValidationError
from .ui.setup_panel import SetupPanel
from .ui.tray import TrayIcon
from .ui.update_banner import UpdateBanner
from .ui.widgets import (
    MicLevelMeter,
    NoWheelComboBox,
    WrapLabel,
    build_language_picker,
    code_from_language_combo,
    set_language_combo_value,
    set_language_picker_placeholder,
)

log = logging.getLogger(__name__)

APP_FONT_SIZE_PT = 11
HOTKEY_TRANSLATION_ID = 0x6100
HOTKEY_SUBTITLES_ID = 0x6101
HOTKEY_TRANSLATION_HOLD_ID = 0x6102
_APP_FONT_FAMILIES: dict[str, str] = {}


def _device_names() -> tuple[list[str], list[str]]:
    from .audio.devices import device_names
    return device_names()


def _install_app_font(app: QtWidgets.QApplication, lang: str = "") -> None:
    for key, filename in (
        ("ko", "NotoSansCJKkr-Regular.otf"),
        ("ko_bold", "NotoSansCJKkr-Bold.otf"),
        ("zh", "NotoSansCJKsc-Regular.otf"),
        ("zh_bold", "NotoSansCJKsc-Bold.otf"),
        ("ja", "PretendardJP-Regular.otf"),
        ("ja_bold", "PretendardJP-Bold.otf"),
    ):
        token = bundled_font(filename)
        fallback = filename
        path = resolve_font_path(token, fallback)
        font_id = QtGui.QFontDatabase.addApplicationFont(path)
        if font_id < 0:
            log.warning("failed to load app font: %s", path)
            continue
        families = QtGui.QFontDatabase.applicationFontFamilies(font_id)
        if families:
            _APP_FONT_FAMILIES[key] = families[0]
    _apply_app_font(app, lang)


def _apply_app_font(app: QtWidgets.QApplication | None, lang: str = "") -> None:
    if app is None:
        return
    lang = i18n.detect(lang)
    family = (
        _APP_FONT_FAMILIES.get(lang)
        or _APP_FONT_FAMILIES.get("ko")
        or _APP_FONT_FAMILIES.get("ja")
        or _APP_FONT_FAMILIES.get("zh")
        or app.font().family()
    )
    font = QtGui.QFont(family)
    font.setPointSize(APP_FONT_SIZE_PT)
    font.setStyleStrategy(
        QtGui.QFont.StyleStrategy.PreferQuality
        | QtGui.QFont.StyleStrategy.PreferAntialias
        | QtGui.QFont.StyleStrategy.ContextFontMerging
    )
    font.setHintingPreference(QtGui.QFont.HintingPreference.PreferNoHinting)
    app.setFont(font)


ENGINE_NAMES = {"gemini": "Gemini", "qwen": "Qwen", "openai": "OpenAI",
                "soniox": "Soniox"}


class _UiSignals(QtCore.QObject):
    refresh = QtCore.Signal()
    toast = QtCore.Signal(str)
    save_done = QtCore.Signal(bool)
    mode_done = QtCore.Signal(bool)
    engine_done = QtCore.Signal(bool)
    device_done = QtCore.Signal(bool)
    speaker_context_done = QtCore.Signal(bool)
    reset_done = QtCore.Signal(bool)
    devices_reloaded = QtCore.Signal(bool)
    test_done = QtCore.Signal(bool, str)
    cable_progress = QtCore.Signal(str)
    cable_done = QtCore.Signal(str, str)   # (error kind or "", detail)
    translation_hotkey = QtCore.Signal()
    subtitles_hotkey = QtCore.Signal()
    translation_hold = QtCore.Signal(bool)


class MainWindow(QtWidgets.QMainWindow):
    def __init__(self, controller, log_file: Path):
        super().__init__()
        self._controller = controller
        self._log_file = Path(log_file)
        self._quitting = False
        self._i18n_widgets = {}
        self._i18n_groups = {}
        self._i18n_tooltips = {}
        self._i18n_calls: list = []
        self._tab_dashboard_idx = -1
        self._tab_settings_idx = -1
        self._tab_logs_idx = -1
        self._last_ui_lang = ""
        self._last_update_result = None
        self._last_config_revision = getattr(controller, "config_revision", 0)
        self._hotkey_signature = None
        self._hotkeys = WindowsGlobalHotkeys()
        self._save_thread = None
        self._mode_thread = None
        self._device_thread = None
        self._speaker_context_thread = None
        self._reset_thread = None
        self._test_thread = None
        self._app_mode_applying = False
        self._device_applying = False
        self._speaker_context_applying = False
        self._devices_reloading = False
        self._setup_checked_at = 0.0
        self._setup_env = None
        self._autofix_note = ""
        self._cable_thread = None
        self._cable_followup = False
        self._cable_progress_dialog = None
        self._cable_box = None
        self._autofix_undo = None   # {"cfg": {path: old}, "vrchat": {name: old}}
        self._app_mode_buttons = {}
        self._inputs, self._outputs = _device_names()
        self._refresh_pending = QtCore.QTimer(self)
        self._refresh_pending.setSingleShot(True)
        self._refresh_pending.setInterval(40)
        self._refresh_pending.timeout.connect(self._refresh)
        self._signals = _UiSignals()
        self._signals.refresh.connect(self._queue_refresh)
        self._signals.toast.connect(self._show_toast)
        self._signals.save_done.connect(self._save_done)
        self._signals.mode_done.connect(self._mode_done)
        self._signals.engine_done.connect(self._engine_done)
        self._signals.device_done.connect(self._device_done)
        self._signals.speaker_context_done.connect(self._speaker_context_done)
        self._signals.reset_done.connect(self._reset_done)
        self._signals.devices_reloaded.connect(self._devices_reloaded)
        self._signals.test_done.connect(self._test_done)
        self._signals.cable_progress.connect(self._cable_progress)
        self._signals.cable_done.connect(self._cable_done)
        self._signals.translation_hotkey.connect(self._toggle_translation)
        self._signals.subtitles_hotkey.connect(self._toggle_subtitles)
        self._signals.translation_hold.connect(self._controller.set_hold_mute)

        self.setWindowTitle(f"VRCLT v{__version__}")
        self._fit_to_screen()
        self._tabs = QtWidgets.QTabWidget()
        self._tabs.setDocumentMode(True)
        self.setCentralWidget(self._tabs)
        # global view switch at the tab bar's right end, clear of the
        # crowded dashboard header
        view_toggle = self._build_view_mode_toggle()
        view_toggle.setContentsMargins(0, 4, 12, 4)
        self._tabs.setCornerWidget(view_toggle, QtCore.Qt.Corner.TopRightCorner)

        self._build_dashboard()
        self._build_settings()
        self._build_logs()
        self._build_tray()
        self._apply_style()
        self._desktop_overlay = DesktopSubtitleOverlay(controller)

        self._timer = QtCore.QTimer(self)
        self._timer.timeout.connect(self._refresh)
        self._timer.start(250)
        # the mic meter needs a faster tick than the 250 ms refresh; runs
        # only while the dashboard tab is visible (gated in _refresh)
        self._meter_timer = QtCore.QTimer(self)
        self._meter_timer.setInterval(75)
        self._meter_timer.timeout.connect(self._tick_mic_meter)
        self._controller.subscribe(self._signals.refresh.emit)
        # worker threads only format a string here; tr() is a pure dict
        # lookup and the signal marshals delivery to the Qt thread
        self._controller.subscribe_errors(
            lambda key, detail: self._signals.toast.emit(
                f"{self._tr(key)}: {detail}"))
        self._sync_hotkeys()
        self._refresh()
        self._update_banner.result_ready.connect(self._on_update_result)
        # VRCLT_NO_UPDATE_CHECK: tests and offline use skip the automatic check
        if not os.environ.get("VRCLT_NO_UPDATE_CHECK"):
            self._update_banner.start_check(__version__)
        QtCore.QTimer.singleShot(1200, self._maybe_prompt_config_reset_after_update)

    # ---------------- construction ----------------
    def _lang(self) -> str:
        try:
            return self._controller.state.ui_lang
        except Exception:
            return i18n.detect(self._controller.cfg.get("ui", {}).get("lang", ""))

    def _tr(self, key: str) -> str:
        return i18n.tr(self._lang(), key)

    def _label(self, key: str) -> QtWidgets.QLabel:
        label = QtWidgets.QLabel(self._tr(key))
        self._i18n_widgets[key] = label
        return label

    def _group(self, key: str) -> QtWidgets.QGroupBox:
        box = QtWidgets.QGroupBox(self._tr(key))
        self._i18n_groups[key] = box
        return box

    def _tip(self, widget, key: str) -> None:
        """Register a retranslatable tooltip; skipped when the key is missing
        (tr() would show the raw key to the user)."""
        if not i18n.has(key):
            return
        widget.setToolTip(self._tr(key))
        self._i18n_tooltips.setdefault(key, []).append(widget)

    def _error_label(self, error: str) -> str:
        if error == "API key is empty.":
            return self._tr("err_api_key_empty")
        if error == "API key must be a Gemini API key, not a URL.":
            return self._tr("err_api_key_url")
        if error == "Qwen (DashScope) API key is empty.":
            return self._tr("err_qwen_api_key_empty")
        if error == "API key must be a DashScope API key, not a URL.":
            return self._tr("err_qwen_api_key_url")
        if error == "Qwen intl endpoint requires a Model Studio workspace ID.":
            return self._tr("err_qwen_workspace_required")
        if error == "OpenAI API key is empty.":
            return self._tr("err_openai_api_key_empty")
        if error == "API key must be an OpenAI API key, not a URL.":
            return self._tr("err_openai_api_key_url")
        if error == "Soniox API key is empty.":
            return self._tr("err_soniox_api_key_empty")
        if error == "API key must be a Soniox API key, not a URL.":
            return self._tr("err_soniox_api_key_url")
        return error

    def _on_retranslate(self, fn) -> None:
        """Apply fn now and re-apply it on every UI-language change.
        Registering at the construction site replaces the old hasattr-guarded
        blocks in _apply_i18n."""
        fn()
        self._i18n_calls.append(fn)

    def _apply_i18n(self) -> None:
        for key, widget in self._i18n_widgets.items():
            widget.setText(self._tr(key))
        for key, box in self._i18n_groups.items():
            box.setTitle(self._tr(key))
        for key, widgets in self._i18n_tooltips.items():
            for widget in widgets:
                widget.setToolTip(self._tr(key))
        for fn in self._i18n_calls:
            fn()

    def _build_dashboard(self) -> None:
        page = QtWidgets.QWidget()
        root = QtWidgets.QVBoxLayout(page)
        root.setContentsMargins(24, 20, 24, 20)
        root.setSpacing(14)
        root.setSizeConstraint(QtWidgets.QLayout.SizeConstraint.SetMinimumSize)

        self._build_dash_header(root)
        c = self._build_dash_controls()
        root.addWidget(self._build_dash_mode_group(c))
        root.addLayout(self._build_dash_pipe_groups(c))
        # subtitle size/position and the UI language stay in sight in both
        # views, right below the language cards (they used to sit folded
        # away at the bottom)
        options = QtWidgets.QHBoxLayout()
        options.setSpacing(16)
        options.addWidget(self._build_dash_display_group(), 3)
        options.addWidget(self._build_dash_app_group(), 2)
        self._options_layout = options
        root.addLayout(options)
        root.addWidget(self._build_dash_audio_group(c))
        root.addWidget(self._dashboard_note)
        self._build_dash_subtitles(root)
        # Small screens and expanded options stay reachable without forcing
        # the entire window taller than the available desktop.
        scroll = QtWidgets.QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QtWidgets.QFrame.Shape.NoFrame)
        scroll.setWidget(page)
        self._dashboard_scroll = scroll
        self._tab_dashboard_idx = self._tabs.addTab(scroll, self._tr("tab_dashboard"))
        self._on_retranslate(lambda: self._tabs.setTabText(
            self._tab_dashboard_idx, self._tr("tab_dashboard")))

    def _build_dash_header(self, root: QtWidgets.QVBoxLayout) -> None:
        """Status row + restart, error label, update/setup banners, toast."""
        top = QtWidgets.QHBoxLayout()
        mark = QtGui.QPixmap(str(icon_path("icon.png")))
        if not mark.isNull():
            brand_icon = QtWidgets.QLabel()
            brand_icon.setFixedSize(32, 32)
            mark = mark.scaled(
                64, 64, QtCore.Qt.AspectRatioMode.KeepAspectRatio,
                QtCore.Qt.TransformationMode.SmoothTransformation)
            mark.setDevicePixelRatio(2)
            brand_icon.setPixmap(mark)
            top.addWidget(brand_icon)
        brand = QtWidgets.QLabel("VRCLT")
        brand.setObjectName("appWordmark")
        top.addWidget(brand)
        top.addSpacing(12)
        self._status_dot = QtWidgets.QLabel()
        self._status_dot.setObjectName("statusDot")
        self._status_dot.setFixedSize(14, 14)
        self._status_text = QtWidgets.QLabel(self._tr("status_stopped"))
        self._status_text.setObjectName("statusText")
        self._status_text.setWordWrap(True)
        self._error_text = QtWidgets.QLabel("")
        self._error_text.setObjectName("errorText")
        self._error_text.setWordWrap(True)
        top.addWidget(self._status_dot)
        # the status takes the spare width: as a wrapped label beside a
        # stretch it got squeezed onto two lines ("Running |" / "Connected")
        top.addWidget(self._status_text, 1)
        self._engine_button = QtWidgets.QPushButton()
        self._engine_button.setObjectName("engineButton")
        self._engine_button.clicked.connect(self._show_engine_settings)
        self._tip(self._engine_button, "dash_engine_tip")
        top.addWidget(self._engine_button)
        self._btn_restart = QtWidgets.QPushButton(self._tr("btn_restart_runtime"))
        self._btn_restart.clicked.connect(self._restart_runtime)
        self._on_retranslate(
            lambda: self._btn_restart.setText(self._tr("btn_restart_runtime")))
        top.addWidget(self._btn_restart)
        root.addLayout(top)
        root.addWidget(self._error_text)

        self._update_banner = UpdateBanner(self._tr, on_available=self._on_update_available)
        self._on_retranslate(self._update_banner.retranslate)
        root.addWidget(self._update_banner)

        self._setup_panel = SetupPanel(
            self._tr,
            on_open_settings=self._show_settings_field,
            on_open_url=QtGui.QDesktopServices.openUrl,
            on_auto_fix=self._on_auto_setup_clicked,
            on_undo=self._undo_auto_setup,
            on_find_mic=self._find_mic,
            on_recheck=lambda: self._sync_setup_panel(force=True),
            on_install_cable=self._install_cable)
        self._on_retranslate(self._setup_panel.retranslate)
        root.addWidget(self._setup_panel)

        self._toast = QtWidgets.QLabel("")
        self._toast.setObjectName("errorText")
        self._toast.setWordWrap(True)
        self._toast.hide()
        root.addWidget(self._toast)
        self._toast_timer = QtCore.QTimer(self)
        self._toast_timer.setSingleShot(True)
        self._toast_timer.timeout.connect(self._toast.hide)

    def _show_engine_settings(self) -> None:
        self._show_settings_field("provider")

    def _show_settings_field(self, path: str) -> None:
        self._tabs.setCurrentIndex(self._tab_settings_idx)
        self._settings_search.clear()
        self._settings_form.focus_field(path)
        field = self._settings_form._fields.get(path)
        if field:
            self._settings_scroll.ensureWidgetVisible(field[0])

    def _build_view_mode_toggle(self) -> QtWidgets.QWidget:
        """Easy view keeps the essentials; full view shows every control."""
        wrap = QtWidgets.QWidget()
        layout = QtWidgets.QHBoxLayout(wrap)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(4)
        group = QtWidgets.QButtonGroup(self)
        group.setExclusive(True)
        self._view_buttons = {}
        for easy, key in ((True, "btn_view_easy"), (False, "btn_view_full")):
            button = QtWidgets.QPushButton(self._tr(key))
            button.setCheckable(True)
            button.setProperty("modeButton", True)
            button.clicked.connect(lambda _checked=False, e=easy: self._set_easy_mode(e))
            self._on_retranslate(lambda b=button, k=key: b.setText(self._tr(k)))
            group.addButton(button)
            layout.addWidget(button)
            self._view_buttons[easy] = button
        self._tip(wrap, "tip_view_mode")
        return wrap

    def _easy_mode(self) -> bool:
        return bool(self._controller.cfg.get("ui", {}).get("easy_mode", True))

    def _set_easy_mode(self, easy: bool) -> None:
        if easy != self._easy_mode():
            setter = getattr(self._controller, "set_easy_mode", None)
            if setter is not None:
                setter(easy)
            else:
                self._controller.cfg.setdefault("ui", {})["easy_mode"] = easy
        self._apply_view_mode()

    def _apply_view_mode(self) -> None:
        """The simple view keeps the setup check, the app choice with VRChat's
        text-only switch, runtime restart, on/off, languages, a mic level,
        subtitle size and position, the UI language and the live subtitles;
        devices, the engine and rarely used options live in the full view."""
        easy = self._easy_mode()
        for value, button in self._view_buttons.items():
            if button.isChecked() != (value == easy):
                button.setChecked(value == easy)
        for widget in (self._out_language_options,
                       self._in_language_options, self._audio_output_column,
                       self._close_action_label, self._close_action,
                       self._out_engine_label, self._out_engine,
                       self._in_engine_label, self._in_engine,
                       self._audio_group, self._engine_button):
            if widget.isHidden() != easy:
                widget.setVisible(not easy)
        for widget in (self._easy_mic_device_label, self._easy_mic_wrap,
                       self._easy_mic_label, self._easy_mic_meter):
            if widget.isHidden() == easy:
                widget.setVisible(easy)
        self._sync_source_pickers()
        self._sync_speaker_context()
        self._layout_dashboard_rows()

    def _fit_to_screen(self) -> None:
        """Never open taller or wider than the screen's work area: a window
        taller than the screen hides the bottom of every tab (including the
        Settings save button) under the taskbar, where scrolling cannot help."""
        screen = self.screen() or QtWidgets.QApplication.primaryScreen()
        width, height = 1040, 820
        if screen is None:
            self.setMinimumSize(780, 560)
            self.resize(width, height)
            return
        area = screen.availableGeometry()
        frame = 40   # title bar and borders sit outside resize()
        self.setMinimumSize(min(780, area.width()), min(520, area.height() - frame))
        width = max(self.minimumWidth(), min(width, area.width() - 24))
        height = max(self.minimumHeight(), min(height, area.height() - frame - 16))
        self.resize(width, height)
        self.move(area.left() + max(0, (area.width() - width) // 2),
                  area.top() + max(0, (area.height() - height - frame) // 2))

    def _disclosure(self, key: str, content: QtWidgets.QWidget) -> QtWidgets.QWidget:
        """Keep occasional controls available without crowding the live view."""
        wrap = QtWidgets.QWidget()
        layout = QtWidgets.QVBoxLayout(wrap)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(8)
        button = QtWidgets.QToolButton()
        button.setObjectName("disclosureButton")
        button.setToolButtonStyle(QtCore.Qt.ToolButtonStyle.ToolButtonTextBesideIcon)
        button.setArrowType(QtCore.Qt.ArrowType.RightArrow)
        button.setCheckable(True)
        button.setSizePolicy(QtWidgets.QSizePolicy.Policy.Expanding,
                             QtWidgets.QSizePolicy.Policy.Fixed)
        self._on_retranslate(lambda: button.setText(self._tr(key)))
        content.hide()

        def toggle(expanded):
            content.setVisible(expanded)
            button.setArrowType(QtCore.Qt.ArrowType.DownArrow if expanded
                                else QtCore.Qt.ArrowType.RightArrow)

        button.toggled.connect(toggle)
        layout.addWidget(button)
        layout.addWidget(content)
        return wrap

    def _build_dash_controls(self) -> dict:
        """Create every dashboard control widget (order preserved from the
        original monolithic builder); returns the container widgets the
        group builders lay out."""
        self._btn_trans = QtWidgets.QPushButton()
        self._btn_trans.setObjectName("transToggle")
        self._btn_trans.clicked.connect(
            lambda: self._controller.set_translation_on(not self._controller.state.translation_on))
        self._btn_sub = QtWidgets.QPushButton()
        self._btn_sub.setObjectName("subToggle")
        self._btn_sub.clicked.connect(
            lambda: self._controller.set_subtitles_on(not self._controller.state.subtitles_on))
        self._out_lang = NoWheelComboBox()
        self._sub_lang = NoWheelComboBox()
        self._ui_lang = NoWheelComboBox()
        self._ui_lang.addItems([i18n.UI_LANG_LABELS[c] for c in i18n.LANGS])
        self._out_lang.currentTextChanged.connect(self._pick_out_lang)
        self._sub_lang.currentTextChanged.connect(self._pick_sub_lang)
        self._ui_lang.currentTextChanged.connect(self._pick_ui_lang)
        self._mic_device = self._build_dashboard_device_combo(
            "outbound.mic_device", self._mic_choices(),
            self._controller.cfg.get("outbound", {}).get("mic_device", ""))
        self._voice_out_device = self._build_dashboard_device_combo(
            "outbound.tts_device", self._outputs,
            self._controller.cfg.get("outbound", {}).get("tts_device", ""))
        self._out_lang_add = build_language_picker(self._tr("ph_out_add"))
        self._out_lang_add.lineEdit().returnPressed.connect(self._add_output_language_from_input)
        self._on_retranslate(lambda: set_language_picker_placeholder(
            self._out_lang_add, self._tr("ph_out_add")))
        self._out_lang_add_btn = QtWidgets.QPushButton(self._tr("btn_add"))
        self._out_lang_add_btn.clicked.connect(self._add_output_language_from_input)
        self._on_retranslate(
            lambda: self._out_lang_add_btn.setText(self._tr("btn_add")))
        self._sub_lang_add = build_language_picker(self._tr("ph_sub_add"))
        self._sub_lang_add.lineEdit().returnPressed.connect(self._add_inbound_language_from_input)
        self._on_retranslate(lambda: set_language_picker_placeholder(
            self._sub_lang_add, self._tr("ph_sub_add")))
        self._sub_lang_add_btn = QtWidgets.QPushButton(self._tr("btn_add"))
        self._sub_lang_add_btn.clicked.connect(self._add_inbound_language_from_input)
        self._on_retranslate(
            lambda: self._sub_lang_add_btn.setText(self._tr("btn_add")))
        out_lang_add_widget = self._build_language_add_control(
            self._out_lang_add, self._out_lang_add_btn)
        sub_lang_add_widget = self._build_language_add_control(
            self._sub_lang_add, self._sub_lang_add_btn)
        app_mode_widget = self._build_app_mode_toggle()
        self._text_only = QtWidgets.QCheckBox(self._tr("btn_text_only_on"))
        self._text_only.toggled.connect(self._apply_text_only)
        self._on_retranslate(
            lambda: self._text_only.setText(self._tr("btn_text_only_on")))
        self._overlay_font_size = QtWidgets.QSpinBox()
        self._overlay_font_size.setRange(config_mod.OVERLAY_FONT_MIN,
                                         config_mod.OVERLAY_FONT_MAX)
        self._overlay_font_size.setSuffix(" px")
        self._overlay_font_size.setValue(
            int(self._controller.cfg.get("overlay", {}).get("font_size", 27)))
        self._overlay_font_size.valueChanged.connect(self._set_overlay_font_size)
        # Full view: the translation and the subtitle engine can differ
        self._out_engine = NoWheelComboBox()
        self._in_engine = NoWheelComboBox()
        self._fill_engine_pickers()
        self._on_retranslate(self._fill_engine_pickers)
        self._out_engine.activated.connect(lambda _index: self._apply_engines())
        self._in_engine.activated.connect(lambda _index: self._apply_engines())
        self._close_action = NoWheelComboBox()
        self._on_retranslate(self._sync_close_action)
        self._close_action.currentIndexChanged.connect(self._pick_close_action)
        self._dashboard_note = QtWidgets.QLabel("")
        self._dashboard_note.setObjectName("noteText")
        self._dashboard_note.setWordWrap(True)
        self._btn_overlay_move = QtWidgets.QPushButton()
        self._btn_overlay_move.setObjectName("overlayMoveBtn")
        self._btn_overlay_move.clicked.connect(self._toggle_overlay_move)
        self._btn_overlay_reset = QtWidgets.QPushButton(self._tr("btn_overlay_reset"))
        self._btn_overlay_reset.clicked.connect(self._reset_overlay_position)
        self._on_retranslate(
            lambda: self._btn_overlay_reset.setText(self._tr("btn_overlay_reset")))
        self._btn_test_out = QtWidgets.QPushButton(self._tr("btn_test_output"))
        self._btn_test_out.setFixedWidth(72)
        self._btn_test_out.clicked.connect(self._test_output_device)
        self._on_retranslate(
            lambda: self._btn_test_out.setText(self._tr("btn_test_output")))
        out_device_wrap = QtWidgets.QWidget()
        out_device_layout = QtWidgets.QHBoxLayout(out_device_wrap)
        out_device_layout.setContentsMargins(0, 0, 0, 0)
        out_device_layout.setSpacing(6)
        out_device_layout.addWidget(self._voice_out_device, 1)
        out_device_layout.addWidget(self._btn_test_out)
        self._btn_find_mic = QtWidgets.QPushButton(self._tr("btn_find_mic"))
        self._btn_find_mic.clicked.connect(self._find_mic)
        self._on_retranslate(
            lambda: self._btn_find_mic.setText(self._tr("btn_find_mic")))
        # the simple view's own copy, inside "my speech" (one widget cannot
        # sit in both views); both edit outbound.mic_device
        self._easy_mic_device = self._build_dashboard_device_combo(
            "outbound.mic_device", self._mic_choices(),
            self._controller.cfg.get("outbound", {}).get("mic_device", ""))
        self._btn_easy_find_mic = QtWidgets.QPushButton(self._tr("btn_find_mic"))
        self._btn_easy_find_mic.clicked.connect(self._find_mic)
        self._on_retranslate(
            lambda: self._btn_easy_find_mic.setText(self._tr("btn_find_mic")))
        self._easy_mic_wrap = QtWidgets.QWidget()
        easy_mic_layout = QtWidgets.QHBoxLayout(self._easy_mic_wrap)
        easy_mic_layout.setContentsMargins(0, 0, 0, 0)
        easy_mic_layout.setSpacing(6)
        easy_mic_layout.addWidget(self._easy_mic_device, 1)
        easy_mic_layout.addWidget(self._btn_easy_find_mic)
        mic_device_wrap = QtWidgets.QWidget()
        mic_device_layout = QtWidgets.QHBoxLayout(mic_device_wrap)
        mic_device_layout.setContentsMargins(0, 0, 0, 0)
        mic_device_layout.setSpacing(6)
        mic_device_layout.addWidget(self._mic_device, 1)
        mic_device_layout.addWidget(self._btn_find_mic)
        tts_gain_widget = self._build_tts_gain_control()
        self._mic_meter = MicLevelMeter()
        # source languages (Qwen only - Gemini auto-detects, combos disabled)
        self._src_lang = build_language_picker(self._tr("ph_src_auto"))
        self._src_lang.activated.connect(lambda _i: self._pick_src_lang())
        self._src_lang.lineEdit().returnPressed.connect(self._pick_src_lang)
        self._on_retranslate(lambda: set_language_picker_placeholder(
            self._src_lang, self._tr("ph_src_auto")))
        self._in_src_lang = build_language_picker(self._tr("ph_src_auto"))
        self._in_src_lang.activated.connect(lambda _i: self._pick_in_src_lang())
        self._in_src_lang.lineEdit().returnPressed.connect(self._pick_in_src_lang)
        self._on_retranslate(lambda: set_language_picker_placeholder(
            self._in_src_lang, self._tr("ph_src_auto")))
        src_label = self._label("label_src_lang")
        in_src_label = self._label("label_in_src_lang")
        mic_label = self._label("label_mic_level")

        self._tip(app_mode_widget, "tip_app_mode")
        self._tip(self._text_only, "tip_text_only")
        self._tip(self._btn_trans, "tip_translate_toggle")
        self._tip(self._btn_sub, "tip_subtitles_toggle")
        self._tip(self._out_lang, "tip_out_lang")
        self._tip(self._sub_lang, "tip_sub_lang")
        self._tip(out_lang_add_widget, "tip_add_out_lang")
        self._tip(sub_lang_add_widget, "tip_add_sub_lang")
        self._tip(self._mic_device, "tip_mic_device")
        self._tip(self._voice_out_device, "tip_voice_out_device")
        self._tip(self._btn_test_out, "tip_test_output")
        self._tip(self._btn_find_mic, "tip_find_mic")
        self._tip(self._btn_easy_find_mic, "tip_find_mic")
        self._tip(self._easy_mic_device, "tip_mic_device")
        self._tip(tts_gain_widget, "tip_tts_gain")
        self._tip(mic_label, "tip_mic_level")
        self._tip(self._mic_meter, "tip_mic_level")
        self._tip(src_label, "tip_src_lang")
        self._tip(self._src_lang, "tip_src_lang")
        self._tip(in_src_label, "tip_src_lang")
        self._tip(self._in_src_lang, "tip_src_lang")
        self._tip(self._overlay_font_size, "tip_pc_sub_size")
        self._tip(self._btn_overlay_move, "tip_overlay_move")
        self._tip(self._close_action, "tip_close_action")
        self._tip(self._out_engine, "tip_out_engine")
        self._tip(self._in_engine, "f.inbound.provider.tip")

        for combo in (self._out_lang, self._sub_lang, self._src_lang,
                      self._in_src_lang, self._out_lang_add, self._sub_lang_add,
                      self._mic_device, self._voice_out_device, self._easy_mic_device):
            combo.setMinimumContentsLength(12)
            combo.setSizeAdjustPolicy(
                QtWidgets.QComboBox.SizeAdjustPolicy.AdjustToMinimumContentsLengthWithIcon)

        return {
            "app_mode": app_mode_widget,
            "out_add": out_lang_add_widget,
            "sub_add": sub_lang_add_widget,
            "out_device": out_device_wrap,
            "mic_device": mic_device_wrap,
            "tts_gain": tts_gain_widget,
            "src_label": src_label,
            "in_src_label": in_src_label,
            "mic_label": mic_label,
        }

    def _build_dash_mode_group(self, c: dict) -> QtWidgets.QGroupBox:
        grp_mode = self._group("dash_grp_mode")
        mode_lay = QtWidgets.QHBoxLayout(grp_mode)
        mode_lay.addWidget(self._label("label_app_mode"))
        mode_lay.addWidget(c["app_mode"], 1)
        mode_lay.addWidget(self._text_only)
        self._mode_group = grp_mode
        return grp_mode

    def _build_dash_pipe_groups(self, c: dict) -> QtWidgets.QHBoxLayout:
        """My speech -> others / others -> me group boxes, side by side."""
        grp_out = self._group("dash_grp_out")
        grp_out.setObjectName("outboundPanel")
        out_lay = QtWidgets.QGridLayout(grp_out)
        out_lay.setHorizontalSpacing(12)
        out_lay.setVerticalSpacing(12)
        out_lay.setSizeConstraint(QtWidgets.QLayout.SizeConstraint.SetMinimumSize)
        out_lay.addWidget(self._btn_trans, 0, 0, 1, 2)
        out_lay.addWidget(self._label("label_out_lang"), 1, 0)
        out_lay.addWidget(self._out_lang, 1, 1)
        out_lay.addWidget(c["src_label"], 2, 0)
        out_lay.addWidget(self._src_lang, 2, 1)
        self._out_language_options = self._disclosure("dash_language_options", c["out_add"])
        out_lay.addWidget(self._out_language_options, 3, 0, 1, 2)
        self._easy_mic_device_label = self._label("label_mic_device")
        out_lay.addWidget(self._easy_mic_device_label, 4, 0)
        out_lay.addWidget(self._easy_mic_wrap, 4, 1)
        self._easy_mic_label = self._label("label_mic_level")
        self._easy_mic_meter = MicLevelMeter()
        self._tip(self._easy_mic_meter, "tip_mic_level")
        out_lay.addWidget(self._easy_mic_label, 5, 0)
        out_lay.addWidget(self._easy_mic_meter, 5, 1)
        self._out_engine_label = self._label("label_out_engine")
        out_lay.addWidget(self._out_engine_label, 6, 0)
        out_lay.addWidget(self._out_engine, 6, 1)
        out_lay.setRowStretch(7, 1)   # spare height below, so both cards align
        out_lay.setColumnStretch(1, 1)

        grp_in = self._group("dash_grp_in")
        grp_in.setObjectName("inboundPanel")
        in_lay = QtWidgets.QGridLayout(grp_in)
        in_lay.setHorizontalSpacing(12)
        in_lay.setVerticalSpacing(12)
        in_lay.setSizeConstraint(QtWidgets.QLayout.SizeConstraint.SetMinimumSize)
        in_lay.addWidget(self._btn_sub, 0, 0, 1, 2)
        in_lay.addWidget(self._label("label_sub_lang"), 1, 0)
        in_lay.addWidget(self._sub_lang, 1, 1)
        in_lay.addWidget(c["in_src_label"], 2, 0)
        in_lay.addWidget(self._in_src_lang, 2, 1)
        self._in_language_options = self._disclosure("dash_language_options", c["sub_add"])
        in_lay.addWidget(self._in_language_options, 3, 0, 1, 2)
        self._in_engine_label = self._label("label_in_engine")
        in_lay.addWidget(self._in_engine_label, 4, 0)
        in_lay.addWidget(self._in_engine, 4, 1)
        self._source_language_labels = (c["src_label"], c["in_src_label"])
        in_lay.setColumnStretch(1, 1)
        in_lay.setRowStretch(6, 1)

        pipes = QtWidgets.QHBoxLayout()
        pipes.setSpacing(16)
        pipes.addWidget(grp_out, 1)
        pipes.addWidget(grp_in, 1)
        self._pipe_layout = pipes
        return pipes

    def _build_dash_audio_group(self, c: dict) -> QtWidgets.QGroupBox:
        grp_audio = self._group("dash_grp_audio")
        self._audio_group = grp_audio
        outer = QtWidgets.QVBoxLayout(grp_audio)
        outer.setSpacing(10)
        columns = QtWidgets.QWidget()
        audio_lay = QtWidgets.QHBoxLayout(columns)
        audio_lay.setContentsMargins(0, 0, 0, 0)
        audio_lay.setSpacing(24)
        audio_lay.setSizeConstraint(QtWidgets.QLayout.SizeConstraint.SetMinimumSize)
        outer.addWidget(columns)
        # VB-Cable's names point the "wrong" way round from the app's view:
        # spell out where the translated voice goes and what the game must use
        self._route_hint = WrapLabel()
        self._route_hint.setObjectName("routeHint")
        self._route_hint.hide()
        outer.addWidget(self._route_hint)
        for index, rows in enumerate((
            ((self._label("label_mic_device"), c["mic_device"]),
             (c["mic_label"], self._mic_meter)),
            ((self._label("label_voice_out_device"), c["out_device"]),
             (self._label("label_tts_gain"), c["tts_gain"])),
        )):
            column = QtWidgets.QWidget()
            grid = QtWidgets.QGridLayout(column)
            grid.setContentsMargins(0, 0, 0, 0)
            grid.setHorizontalSpacing(12)
            grid.setVerticalSpacing(12)
            grid.setSizeConstraint(QtWidgets.QLayout.SizeConstraint.SetMinimumSize)
            for row, (label, control) in enumerate(rows):
                grid.addWidget(label, row, 0)
                grid.addWidget(control, row, 1)
            grid.setColumnStretch(1, 1)
            audio_lay.addWidget(column, 1)
            if index == 1:
                self._audio_output_column = column
        self._audio_layout = audio_lay
        return grp_audio

    def _build_dash_display_group(self) -> QtWidgets.QGroupBox:
        grp_display = self._group("dash_grp_display")
        disp_lay = QtWidgets.QHBoxLayout(grp_display)
        disp_lay.addWidget(self._label("label_pc_sub_size"))
        disp_lay.addWidget(self._overlay_font_size)
        disp_lay.addStretch(1)
        disp_lay.addWidget(self._btn_overlay_move)
        disp_lay.addWidget(self._btn_overlay_reset)
        self._display_layout = disp_lay
        return grp_display

    def _build_dash_app_group(self) -> QtWidgets.QGroupBox:
        grp_app = self._group("dash_grp_app")
        app_lay = QtWidgets.QHBoxLayout(grp_app)
        app_lay.setSpacing(18)
        app_lay.addWidget(self._label("ui_lang"))
        app_lay.addWidget(self._ui_lang)
        self._close_action_label = self._label("label_close_action")
        app_lay.addWidget(self._close_action_label)
        app_lay.addWidget(self._close_action)
        app_lay.addStretch(1)
        self._app_layout = app_lay
        return grp_app

    def _build_dash_subtitles(self, root: QtWidgets.QVBoxLayout) -> None:
        title = self._label("dash_live_preview")
        title.setObjectName("sectionTitle")
        heading = QtWidgets.QHBoxLayout()
        heading.addWidget(title)
        heading.addStretch(1)
        self._speaker_badge = self._label("speaker_diarization_on")
        self._speaker_badge.setObjectName("speakerBadge")
        self._tip(self._speaker_badge, "speaker_diarization_tip")
        heading.addWidget(self._speaker_badge)
        root.addLayout(heading)
        self._keep_speaker_context = QtWidgets.QCheckBox()
        self._on_retranslate(lambda: self._keep_speaker_context.setText(
            self._tr("f.soniox.keep_speaker_context")))
        self._tip(self._keep_speaker_context, "f.soniox.keep_speaker_context.tip")
        self._keep_speaker_context.toggled.connect(self._apply_speaker_context)
        context_row = QtWidgets.QHBoxLayout()
        context_row.setSpacing(16)
        context_row.addWidget(self._keep_speaker_context, 0, QtCore.Qt.AlignmentFlag.AlignLeft)
        self._soniox_idle_hint = QtWidgets.QLabel()
        self._soniox_idle_hint.setObjectName("noteText")
        self._soniox_idle_hint.setWordWrap(False)
        self._soniox_idle_hint.setSizePolicy(QtWidgets.QSizePolicy.Policy.Preferred,
                                            QtWidgets.QSizePolicy.Policy.Fixed)
        context_row.addWidget(self._soniox_idle_hint, 1)
        self._context_layout = context_row
        root.addLayout(context_row)
        self._subtitle_view = QtWidgets.QTextEdit()
        self._subtitle_view.setObjectName("subtitleView")
        self._subtitle_view.setReadOnly(True)
        self._subtitle_view.setMinimumHeight(140)
        self._subtitle_view.setSizePolicy(QtWidgets.QSizePolicy.Policy.Expanding,
                                         QtWidgets.QSizePolicy.Policy.Ignored)
        self._on_retranslate(lambda: self._subtitle_view.setPlaceholderText(
            self._tr("subtitle_live_placeholder")))
        self._subtitle_snapshot = None
        root.addWidget(self._subtitle_view, 1)

    def _build_tts_gain_control(self) -> QtWidgets.QWidget:
        self._tts_gain_slider = QtWidgets.QSlider(QtCore.Qt.Orientation.Horizontal)
        self._tts_gain_slider.setRange(0, 200)
        self._tts_gain_slider.setPageStep(10)
        self._tts_gain_slider.setValue(round(self._controller.tts_gain() * 100))
        self._tts_gain_label = QtWidgets.QLabel(f"{self._tts_gain_slider.value()}%")
        self._tts_gain_label.setFixedWidth(48)
        # debounce: live label instantly, save+apply 200 ms after the last move
        self._tts_gain_timer = QtCore.QTimer(self)
        self._tts_gain_timer.setSingleShot(True)
        self._tts_gain_timer.setInterval(200)
        self._tts_gain_timer.timeout.connect(
            lambda: self._controller.set_tts_gain(self._tts_gain_slider.value() / 100.0))
        self._tts_gain_slider.valueChanged.connect(self._tts_gain_changed)
        wrap = QtWidgets.QWidget()
        layout = QtWidgets.QHBoxLayout(wrap)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(6)
        layout.addWidget(self._tts_gain_slider, 1)
        layout.addWidget(self._tts_gain_label)
        return wrap

    def _tts_gain_changed(self, value: int) -> None:
        self._tts_gain_label.setText(f"{value}%")
        self._tts_gain_timer.start()

    def _tick_mic_meter(self) -> None:
        level = self._controller.mic_level() or (None, None)
        self._mic_meter.set_level(*level)
        self._easy_mic_meter.set_level(*level)

    def _build_dashboard_device_combo(self, path: str, names: list[str], current: str) -> NoWheelComboBox:
        combo = NoWheelComboBox()
        combo.setEditable(True)
        combo.setInsertPolicy(QtWidgets.QComboBox.InsertPolicy.NoInsert)
        combo.setMinimumContentsLength(18)
        self._sync_device_combo(combo, names, current or "")
        combo.activated.connect(lambda _idx, p=path, c=combo: self._apply_dashboard_device(p, c))
        line_edit = combo.lineEdit()
        if line_edit is not None:
            line_edit.returnPressed.connect(
                lambda p=path, c=combo: self._apply_dashboard_device(p, c))
            line_edit.editingFinished.connect(
                lambda p=path, c=combo: self._apply_dashboard_device(p, c))
        return combo

    def _build_language_add_control(self, edit: QtWidgets.QComboBox,
                                    button: QtWidgets.QPushButton) -> QtWidgets.QWidget:
        wrap = QtWidgets.QWidget()
        layout = QtWidgets.QHBoxLayout(wrap)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(6)
        button.setFixedWidth(64)
        layout.addWidget(edit, 1)
        layout.addWidget(button)
        return wrap

    def _build_app_mode_toggle(self) -> QtWidgets.QWidget:
        wrap = QtWidgets.QWidget()
        layout = QtWidgets.QHBoxLayout(wrap)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(8)
        self._app_mode_group = QtWidgets.QButtonGroup(self)
        self._app_mode_group.setExclusive(True)
        # product names stay untranslated; "custom" is a word, so it follows
        # the UI language and is re-read on every language change
        labels = {
            "vrchat": "VRChat",
            "discord": "Discord",
        }
        for mode in config_mod.APP_MODES:
            btn = QtWidgets.QPushButton(labels.get(mode, ""))
            if mode not in labels:
                self._on_retranslate(
                    lambda b=btn, m=mode: b.setText(self._tr(f"app_mode_{m}")))
            btn.setCheckable(True)
            btn.setProperty("modeButton", True)
            btn.setMinimumSize(96, 38)
            btn.clicked.connect(lambda _checked=False, m=mode: self._apply_app_mode(m))
            self._app_mode_group.addButton(btn)
            self._app_mode_buttons[mode] = btn
            layout.addWidget(btn)
        layout.addStretch(1)
        self._set_app_mode_checked(self._controller.cfg.get("app", {}).get("mode", "vrchat"))
        return wrap

    def _build_settings(self) -> None:
        page = QtWidgets.QWidget()
        outer = QtWidgets.QVBoxLayout(page)
        outer.setContentsMargins(0, 0, 0, 0)
        search_row = QtWidgets.QHBoxLayout()
        search_row.setContentsMargins(18, 12, 18, 0)
        self._settings_search = QtWidgets.QLineEdit()
        self._settings_search.setClearButtonEnabled(True)
        self._on_retranslate(lambda: self._settings_search.setPlaceholderText(
            self._tr("settings_search_ph")))
        search_row.addWidget(self._settings_search)
        outer.addLayout(search_row)
        scroll = QtWidgets.QScrollArea()
        scroll.setWidgetResizable(True)
        self._settings_scroll = scroll
        body = QtWidgets.QWidget()
        self._settings_layout = QtWidgets.QVBoxLayout(body)
        self._settings_layout.setContentsMargins(18, 18, 18, 18)
        self._settings_layout.setSpacing(12)
        scroll.setWidget(body)
        outer.addWidget(scroll, 1)

        buttons = QtWidgets.QHBoxLayout()
        buttons.setContentsMargins(18, 8, 18, 16)
        self._btn_devices = QtWidgets.QPushButton(self._tr("btn_refresh_devices"))
        self._btn_devices.clicked.connect(self._reload_devices)
        self._on_retranslate(
            lambda: self._btn_devices.setText(self._tr("btn_refresh_devices")))
        self._btn_reset_config = QtWidgets.QPushButton(self._tr("btn_reset_config"))
        self._btn_reset_config.clicked.connect(self._confirm_reset_config)
        self._on_retranslate(
            lambda: self._btn_reset_config.setText(self._tr("btn_reset_config")))
        self._btn_save = QtWidgets.QPushButton(self._tr("btn_save_restart"))
        self._btn_save.setObjectName("primaryButton")
        self._btn_save.clicked.connect(self._save_settings)
        self._on_retranslate(
            lambda: self._btn_save.setText(self._tr("btn_save_restart")))
        self._settings_note = QtWidgets.QLabel("")
        self._settings_note.setObjectName("noteText")
        buttons.addWidget(self._settings_note, 1)
        buttons.addWidget(self._btn_devices)
        buttons.addWidget(self._btn_reset_config)
        buttons.addWidget(self._btn_save)
        outer.addLayout(buttons)

        self._settings_form = SettingsForm(
            self._controller, self._tr, self._settings_layout,
            get_devices=lambda: (self._mic_choices(), self._outputs),
            on_hotkey_capture_start=self._hotkeys.stop,
            on_hotkey_capture_end=lambda: self._sync_hotkeys(force=True))
        self._settings_search.textChanged.connect(self._settings_form.apply_filter)
        self._settings_form.filter_reset_requested.connect(self._settings_search.clear)
        self._populate_settings()
        self._tab_settings_idx = self._tabs.addTab(page, self._tr("tab_settings"))
        self._on_retranslate(lambda: self._tabs.setTabText(
            self._tab_settings_idx, self._tr("tab_settings")))

    def _build_logs(self) -> None:
        page = QtWidgets.QWidget()
        root = QtWidgets.QVBoxLayout(page)
        root.setContentsMargins(0, 0, 0, 0)
        self._log_panel = LogPanel(self._log_file, self._tr)
        self._on_retranslate(self._log_panel.retranslate)
        root.addWidget(self._log_panel, 1)

        bottom = QtWidgets.QVBoxLayout()
        bottom.setContentsMargins(18, 0, 18, 18)
        self._about_text = QtWidgets.QLabel(
            self._tr("about_paths").format(config=config_mod.CONFIG_PATH)
        )
        self._about_text.setWordWrap(True)
        self._about_text.setTextInteractionFlags(
            QtCore.Qt.TextInteractionFlag.TextSelectableByMouse)
        self._on_retranslate(lambda: self._about_text.setText(
            self._tr("about_paths").format(config=config_mod.CONFIG_PATH)))
        update_row = QtWidgets.QHBoxLayout()
        self._btn_check_update = QtWidgets.QPushButton(self._tr("btn_check_update"))
        self._btn_check_update.clicked.connect(self._check_updates)
        self._update_status = QtWidgets.QLabel(
            self._tr("update_status_idle").format(current=__version__))
        self._update_status.setObjectName("noteText")
        self._update_status.setWordWrap(True)
        self._on_retranslate(lambda: (
            self._btn_check_update.setText(self._tr("btn_check_update")),
            self._render_update_status()))
        update_row.addWidget(self._btn_check_update)
        update_row.addWidget(self._update_status, 1)
        bottom.addLayout(update_row)
        bottom.addWidget(self._about_text)
        root.addLayout(bottom)
        self._tab_logs_idx = self._tabs.addTab(page, self._tr("tab_logs"))
        self._on_retranslate(lambda: self._tabs.setTabText(
            self._tab_logs_idx, self._tr("tab_logs")))

    def _build_tray(self) -> None:
        self._tray = TrayIcon(
            self, self._tr,
            on_show=self._show_main,
            on_show_settings=self._show_settings,
            on_open_update=lambda: self._update_banner.open_release(),
            on_toggle_translation=self._toggle_translation,
            on_toggle_subtitles=self._toggle_subtitles,
            on_quit=self._quit,
            on_message_clicked=lambda: self._update_banner.open_release())
        self._on_retranslate(self._tray.retranslate)

    def _apply_style(self) -> None:
        self.setStyleSheet(theme.build_qss())

    # ---------------- settings form (thin delegates; see ui/settings_form) ----
    def _populate_settings(self) -> None:
        """Rebuild the settings form, restoring scroll position and focus
        (populate() destroys every widget, which used to reset both on each
        save/reset/device-reload/language change)."""
        scroll = getattr(self, "_settings_scroll", None)
        if scroll is None:
            self._settings_form.populate()
            return
        pos = scroll.verticalScrollBar().value()
        focus_path = self._settings_form.focused_field_path()

        self._settings_form.populate()

        def restore():
            scroll.verticalScrollBar().setValue(pos)
            self._settings_form.focus_field(focus_path)

        # let the new layout settle before restoring
        QtCore.QTimer.singleShot(0, restore)

    def _sync_settings_from_config(self) -> None:
        self._settings_form.sync_from_config()

    def _sync_steamvr_autolaunch(self) -> None:
        self._settings_form.sync_steamvr_autolaunch()

    def _settings_from_fields(self) -> dict:
        return self._settings_form.config_from_fields()

    # ---------------- actions ----------------
    def _on_update_available(self, info, message: str) -> None:
        """Fires once (from the banner) when a newer release is found."""
        self._tray.set_update_visible(True)
        self._tray.show_message(self._tr("update_title"), message, 10000)

    def _show_toast(self, text: str, msecs: int = 8000) -> None:
        """Transient dashboard notice for background failures (Qt thread)."""
        self._toast.setText(text)
        self._toast.show()
        self._toast_timer.start(msecs)

    def _check_updates(self) -> None:
        if self._update_banner.start_check(__version__, force=True):
            self._btn_check_update.setEnabled(False)
            self._update_status.setToolTip("")
            self._update_status.setText(self._tr("update_checking"))

    def _on_update_result(self, result) -> None:
        self._last_update_result = result
        self._btn_check_update.setEnabled(True)
        self._render_update_status()

    def _render_update_status(self) -> None:
        result = self._last_update_result
        if result is None:
            self._update_status.setText(
                self._tr("update_status_idle").format(current=__version__))
            return
        self._update_status.setToolTip("")
        if result.status == "update":
            self._update_status.setText(
                self._tr("update_available_short").format(
                    latest=result.info.latest_version, current=__version__))
        elif result.status == "up_to_date":
            self._update_status.setText(
                self._tr("update_up_to_date").format(current=__version__))
        else:
            err_key = f"update_err_{result.error_kind}"
            reason = self._tr(err_key) if i18n.has(err_key) else result.detail
            self._update_status.setText(
                self._tr("update_check_failed").format(reason=reason))
            self._update_status.setToolTip(result.detail)

    def _maybe_prompt_config_reset_after_update(self) -> None:
        previous = self._controller.last_config_version()
        if previous == __version__:
            return
        if not config_mod.CONFIG_PATH.exists():
            self._controller.mark_config_version_seen(__version__)
            return
        body = self._tr("reset_config_update_body").format(
            previous=previous or self._tr("version_unknown"),
            current=__version__,
        )
        reply = QtWidgets.QMessageBox.question(
            self,
            self._tr("reset_config_title"),
            body,
            QtWidgets.QMessageBox.StandardButton.Yes
            | QtWidgets.QMessageBox.StandardButton.No,
            QtWidgets.QMessageBox.StandardButton.No,
        )
        if reply == QtWidgets.QMessageBox.StandardButton.Yes:
            self._run_config_reset()
        else:
            self._controller.mark_config_version_seen(__version__)

    def _confirm_reset_config(self) -> None:
        reply = QtWidgets.QMessageBox.question(
            self,
            self._tr("reset_config_title"),
            self._tr("reset_config_body"),
            QtWidgets.QMessageBox.StandardButton.Yes
            | QtWidgets.QMessageBox.StandardButton.No,
            QtWidgets.QMessageBox.StandardButton.No,
        )
        if reply == QtWidgets.QMessageBox.StandardButton.Yes:
            self._run_config_reset()

    def _spawn_restart(self, thread_name: str, op, done_signal) -> threading.Thread:
        """Run op() (a controller call returning ok) on a daemon worker and
        emit done_signal(ok) back onto the Qt thread."""
        def run():
            done_signal.emit(op())

        t = threading.Thread(target=run, daemon=True, name=thread_name)
        t.start()
        return t

    def _apply_config_async(self, build_cfg, *, fail_key: str, busy_key: str,
                            note: QtWidgets.QLabel, done_signal,
                            thread_name: str, on_busy,
                            on_build_error=None) -> threading.Thread | None:
        """Shared save-and-restart shape for the settings/dashboard handlers.
        build_cfg() runs on the Qt thread and must validate + config_mod.save(),
        returning the cfg to restart with (None = silent no-op). On exception
        the note shows '{fail_key}: {e}' and on_build_error() rolls the
        widgets back; on success the busy note is set, on_busy() disables the
        controls, and the restart runs on a worker via _spawn_restart."""
        try:
            cfg = build_cfg()
        except Exception as e:
            note.setText(f"{self._tr(fail_key)}: {e}")
            if on_build_error is not None:
                on_build_error()
            return None
        if cfg is None:
            return None
        note.setText(self._tr(busy_key))
        on_busy()
        return self._spawn_restart(
            thread_name, lambda: self._controller.restart(cfg), done_signal)

    def _run_config_reset(self) -> None:
        if self._reset_thread is not None and self._reset_thread.is_alive():
            return
        self._settings_note.setText(self._tr("msg_reset_restarting"))
        self._btn_reset_config.setEnabled(False)
        self._btn_save.setEnabled(False)
        self._reset_thread = self._spawn_restart(
            "vrclt-reset",
            lambda: self._controller.reset_config_preserving_language_lists(__version__),
            self._signals.reset_done)

    def _reset_done(self, ok: bool) -> None:
        self._btn_reset_config.setEnabled(True)
        self._btn_save.setEnabled(True)
        self._settings_note.setText(
            self._tr("msg_reset_done") if ok else self._tr("msg_reset_failed"))
        self._sync_hotkeys(force=True)
        self._populate_settings()

    def _save_settings(self) -> None:
        def build():
            try:
                cfg = self._settings_from_fields()
            except SettingsValidationError as e:
                first = self._settings_form.first_invalid_widget()
                if first is not None:
                    self._settings_scroll.ensureWidgetVisible(first)
                fields = ", ".join(self._tr(err.label_key) for err in e.errors)
                raise ValueError(
                    self._tr("msg_invalid_field").format(fields=fields))
            # every engine the runtime will start needs a valid key
            for prov in config_mod.providers_in_use(cfg):
                key_path = "api_key" if prov == "gemini" else f"{prov}.api_key"
                key_error = config_mod.api_key_validation_error(
                    _get_path(cfg, key_path, ""), provider_label={
                        "gemini": "Gemini", "qwen": "DashScope",
                        "openai": "OpenAI", "soniox": "Soniox",
                    }[prov])
                if key_error:
                    self._settings_form.focus_field(key_path)
                    self._settings_scroll.ensureWidgetVisible(
                        self._settings_form._fields[key_path][0])
                    error_key = "err_api_key_url" if prov == "gemini" else f"err_{prov}_api_key_url"
                    raise ValueError(self._tr(error_key))
            qw = cfg.get("qwen", {})
            if "qwen" in config_mod.providers_in_use(cfg) \
                    and str(qw.get("endpoint", "intl") or "intl").strip() != "beijing" \
                    and not str(qw.get("workspace_id", "") or "").strip() \
                    and not str(qw.get("base_url", "") or "").strip():
                raise ValueError(self._tr("err_qwen_workspace_required"))
            force_profile = (
                cfg.get("app", {}).get("mode")
                != self._controller.raw_cfg.get("app", {}).get("mode")
            )
            cfg = config_mod.apply_app_profile(cfg, force=force_profile)
            config_mod.save(cfg)
            return cfg

        thread = self._apply_config_async(
            build, fail_key="msg_save_failed", busy_key="msg_save_restarting",
            note=self._settings_note, done_signal=self._signals.save_done,
            thread_name="vrclt-restart",
            on_busy=lambda: self._btn_save.setEnabled(False))
        if thread is not None:
            self._save_thread = thread

    def _save_done(self, ok: bool) -> None:
        self._btn_save.setEnabled(True)
        self._settings_note.setText(
            self._tr("msg_applied") if ok else self._tr("msg_saved_start_failed"))
        self._sync_hotkeys(force=True)
        self._populate_settings()

    def _restart_runtime(self) -> None:
        self._settings_note.setText(self._tr("msg_runtime_restarting"))
        self._controller.restart_async()

    def _reload_devices(self) -> None:
        """Refresh device lists via a runtime restart with a PortAudio
        reinit (the only way hot-plugged devices become visible)."""
        if self._devices_reloading:
            return
        self._devices_reloading = True
        self._btn_devices.setEnabled(False)
        self._btn_test_out.setEnabled(False)
        self._settings_note.setText(self._tr("msg_devices_refreshing"))
        self._spawn_restart("vrclt-device-reload",
                            self._controller.reinit_audio_devices,
                            self._signals.devices_reloaded)

    def _devices_reloaded(self, ok: bool) -> None:
        self._devices_reloading = False
        self._inputs, self._outputs = _device_names()
        self._btn_devices.setEnabled(True)
        self._btn_test_out.setEnabled(True)
        self._sync_dashboard_devices(force=True)
        self._populate_settings()
        self._sync_setup_panel(force=True)
        self._settings_note.setText(
            self._tr("msg_devices_refreshed") if ok
            else self._tr("msg_saved_start_failed"))
        if self._cable_followup:
            self._cable_followup = False
            loopback = platform_support.default_loopback_device()
            self._close_cable_progress()
            if setup_check.find_device(self._outputs, loopback):
                self._run_auto_setup()
            else:
                self._dashboard_note.setText(self._tr("msg_cable_not_found"))
                self._cable_notice(self._tr("msg_cable_not_found"))
                self._set_auto_setup_pending(True)

    def _test_output_device(self) -> None:
        """Play a short test tone on the (unsaved) output-combo selection."""
        if (self._test_thread is not None and self._test_thread.is_alive()) \
                or self._device_applying or self._devices_reloading:
            return
        value = self._voice_out_device.currentText().strip()
        self._btn_test_out.setEnabled(False)
        self._btn_devices.setEnabled(False)
        self._dashboard_note.setText(self._tr("msg_test_playing"))

        def run():
            try:
                from .audio.devices import sine_test
                sine_test(value, seconds=1.0)
                self._signals.test_done.emit(True, "")
            except Exception as e:
                self._signals.test_done.emit(False, str(e))

        self._test_thread = threading.Thread(
            target=run, daemon=True, name="vrclt-sound-test")
        self._test_thread.start()

    def _test_done(self, ok: bool, error: str) -> None:
        self._btn_test_out.setEnabled(not self._devices_reloading)
        self._btn_devices.setEnabled(not self._devices_reloading)
        self._dashboard_note.setText(
            "" if ok else f"{self._tr('msg_test_failed')}: {error}")

    def _apply_dashboard_device(self, path: str, combo: QtWidgets.QComboBox) -> None:
        if self._device_applying:
            self._sync_dashboard_devices()
            return
        value = combo.currentText().strip()
        current = str(_get_path(self._controller.raw_cfg, path, "") or "")
        if value == current:
            return

        def build():
            cfg = copy.deepcopy(self._controller.raw_cfg)
            _set_path(cfg, path, value)
            cfg = config_mod.apply_app_profile(cfg)
            config_mod.save(cfg)
            return cfg

        def busy():
            self._device_applying = True
            self._set_dashboard_apply_enabled(False)

        thread = self._apply_config_async(
            build, fail_key="msg_device_failed", busy_key="msg_device_applying",
            note=self._dashboard_note, done_signal=self._signals.device_done,
            thread_name="vrclt-device-restart", on_busy=busy,
            on_build_error=self._sync_dashboard_devices)
        if thread is not None:
            self._device_thread = thread

    def _device_done(self, ok: bool) -> None:
        self._device_applying = False
        self._set_dashboard_apply_enabled(True)
        note, self._autofix_note = self._autofix_note, ""
        self._dashboard_note.setText(
            (note or self._tr("msg_applied")) if ok
            else " ".join(filter(None, (note, self._tr("msg_saved_start_failed")))))
        self._sync_dashboard_devices(force=True)
        self._populate_settings()
        self._sync_setup_panel(force=True)

    def _sync_source_pickers(self) -> None:
        """Spoken-language pickers: only where the direction's engine uses
        them, and in the easy view only where it requires them (Qwen 3.5)."""
        cfg = self._controller.cfg
        st = self._controller.state
        out_label, in_label = self._source_language_labels
        for direction, label, combo, code in (
                ("outbound", out_label, self._src_lang, st.source_language),
                ("inbound", in_label, self._in_src_lang, st.inbound_source_language)):
            shown = config_mod.uses_source_language(cfg, direction) and (
                not self._easy_mode() or config_mod.provider_for(cfg, direction) == "qwen")
            label.setVisible(shown)
            combo.setVisible(shown)
            if combo.isEnabled() != shown:
                combo.setEnabled(shown)
            if not combo.hasFocus() and \
                    code_from_language_combo(combo, []) != code:
                blocked = combo.blockSignals(True)
                try:
                    set_language_combo_value(combo, code)
                finally:
                    combo.blockSignals(blocked)

    # ---------------- setup checklist ----------------
    def _collect_setup(self):
        cfg = self._controller.cfg
        osc_status = getattr(self._controller, "osc_status", None)
        return setup_check.collect(
            cfg, inputs=self._inputs, outputs=self._outputs,
            osc_status=osc_status() if osc_status else (None, 0.0))

    def _sync_setup_panel(self, force: bool = False) -> None:
        """Re-run the checklist every few seconds (registry and device
        defaults change outside the app) or right after a change."""
        now = time.monotonic()
        if self._devices_reloading or (
                not force and now - self._setup_checked_at < 2.0):
            return
        self._setup_checked_at = now
        cfg = self._controller.cfg
        try:
            env = self._collect_setup()
            items = setup_check.run_checks(cfg, env)
        except Exception:
            log.exception("setup checklist failed")
            return
        self._setup_env = env
        self._resume_auto_setup(items)
        app = env.app_name or self._tr("setup_app_generic")
        self._setup_panel.sync(
            items, provider=env.missing_key_provider or config_mod.provider(cfg),
            endpoint=str(cfg.get("qwen", {}).get("endpoint", "intl") or "intl").strip(),
            app=app)
        self._set_default_placeholder(self._mic_device, env.default_input)
        self._set_default_placeholder(self._easy_mic_device, env.default_input)
        self._set_default_placeholder(self._voice_out_device, env.default_output)
        loopback = platform_support.default_loopback_device()
        self._route_hint.setVisible(bool(loopback))
        if loopback:
            cable_in = setup_check.find_device(env.outputs, loopback) or loopback
            self._route_hint.setText(self._tr("audio_route_hint").format(
                cable_in=cable_in.split("(", 1)[0].strip(),
                cable_out=setup_check.app_mic_name(cable_in), app=app))

    def _mic_choices(self) -> list[str]:
        """Mic pickers never offer the cable's recording side (a loop)."""
        return setup_check.mic_choices(self._inputs, self._controller.cfg)

    def _set_auto_setup_pending(self, pending: bool) -> None:
        setter = getattr(self._controller, "set_auto_setup_pending", None)
        if setter is not None:
            setter(pending)
        else:
            self._controller.cfg.setdefault("meta", {})["auto_setup_pending"] = pending

    def _resume_auto_setup(self, items) -> None:
        """VB-Cable installed but needed a restart: once its devices are
        there (the next start, usually), finish what Auto setup began."""
        if not self._controller.cfg.get("meta", {}).get("auto_setup_pending") \
                or self._device_applying or self._devices_reloading:
            return
        if any(item.key == "cable" and item.status == setup_check.OK for item in items):
            self._set_auto_setup_pending(False)
            QtCore.QTimer.singleShot(0, self._run_auto_setup)

    def _set_default_placeholder(self, combo: QtWidgets.QComboBox, name: str) -> None:
        """An empty device field means the system default: say which one."""
        edit = combo.lineEdit()
        text = self._tr("device_system_default").format(name=name or "-")
        if edit is not None and edit.placeholderText() != text:
            edit.setPlaceholderText(text)

    def _auto_setup_plan(self):
        env = self._collect_setup()
        new_cfg, changed = setup_check.auto_fix(self._controller.raw_cfg, env)
        return new_cfg, changed, setup_check.vrchat_changes(self._controller.cfg, env)

    def _plan_lines(self, plan) -> list[str]:
        new_cfg, changed, vrchat = plan
        out = new_cfg.get("outbound", {})
        lines = []
        if "outbound.tts_device" in changed:
            lines.append(self._tr("setup_changed_tts").format(name=out.get("tts_device", "")))
        if "outbound.mic_device" in changed:
            lines.append(self._tr("setup_changed_mic").format(name=out.get("mic_device", "")))
        if "mic" in vrchat:
            lines.append(self._tr("setup_changed_vrchat_mic").format(name=vrchat["mic"]))
        if "osc" in vrchat:
            lines.append(self._tr("setup_changed_vrchat_osc"))
        return lines

    def _on_auto_setup_clicked(self) -> None:
        """The button asks first: a stray click must not quietly replace the
        microphone, the voice output or VRChat's settings."""
        if self._device_applying or self._devices_reloading:
            return
        try:
            plan = self._auto_setup_plan()
        except Exception as e:
            log.exception("auto setup failed")
            self._dashboard_note.setText(f"{self._tr('msg_device_failed')}: {e}")
            return
        lines = self._plan_lines(plan)
        if not lines:
            self._dashboard_note.setText(self._tr("setup_autofix_nothing"))
            self._sync_setup_panel(force=True)
            return
        if self._confirm_auto_setup(lines, plan[2]):
            self._run_auto_setup(plan)

    def _confirm_auto_setup(self, lines: list[str], vrchat: dict) -> bool:
        from . import vrchat_settings
        text = "\n".join([self._tr("setup_autofix_confirm"), ""]
                         + [f"• {line}" for line in lines])
        if vrchat and vrchat_settings.running():
            text += "\n\n" + self._tr("setup_vrchat_running")
        text += "\n\n" + self._tr("setup_autofix_undo_hint")
        box = QtWidgets.QMessageBox(self)
        box.setIcon(QtWidgets.QMessageBox.Icon.Question)
        box.setWindowTitle(self._tr("setup_btn_auto_fix"))
        box.setText(text)
        apply = box.addButton(self._tr("setup_autofix_apply"),
                              QtWidgets.QMessageBox.ButtonRole.AcceptRole)
        box.addButton(QtWidgets.QMessageBox.StandardButton.Cancel)
        box.exec()
        return box.clickedButton() is apply

    def _run_auto_setup(self, plan=None) -> None:
        if self._device_applying or self._devices_reloading:
            return
        if plan is None:
            try:
                plan = self._auto_setup_plan()
            except Exception as e:
                log.exception("auto setup failed")
                self._dashboard_note.setText(f"{self._tr('msg_device_failed')}: {e}")
                return
        new_cfg, changed, vrchat = plan
        out = new_cfg.get("outbound", {})
        parts = []
        if "outbound.tts_device" in changed:
            parts.append(self._tr("setup_changed_tts").format(name=out.get("tts_device", "")))
        if "outbound.mic_device" in changed:
            parts.append(self._tr("setup_changed_mic").format(name=out.get("mic_device", "")))
        vrchat_note, vrchat_previous = (self._apply_vrchat_settings(vrchat, parts)
                                        if vrchat else ("", {}))
        undo_cfg = {path: config_mod.get_path(self._controller.raw_cfg, path, "")
                    for path in changed}
        if undo_cfg or vrchat_previous:
            self._autofix_undo = {"cfg": undo_cfg, "vrchat": vrchat_previous}
            self._setup_panel.set_undo_available(True)
        if not parts:
            self._dashboard_note.setText(vrchat_note or self._tr("setup_autofix_nothing"))
            self._sync_setup_panel(force=True)
            return
        summary = " ".join(filter(None, (
            self._tr("setup_autofix_done").format(changes=", ".join(parts)), vrchat_note)))
        if not changed:
            self._dashboard_note.setText(summary)
            self._sync_setup_panel(force=True)
            return

        def build():
            cfg = config_mod.apply_app_profile(copy.deepcopy(new_cfg))
            config_mod.save(cfg)
            return cfg

        def busy():
            self._device_applying = True
            self._set_dashboard_apply_enabled(False)

        thread = self._apply_config_async(
            build, fail_key="msg_device_failed", busy_key="msg_autofix_applying",
            note=self._dashboard_note, done_signal=self._signals.device_done,
            thread_name="vrclt-auto-setup", on_busy=busy,
            on_build_error=self._sync_dashboard_devices)
        if thread is not None:
            self._device_thread = thread
            self._autofix_note = summary

    def _apply_vrchat_settings(self, changes: dict, parts: list) -> tuple[str, dict]:
        """Write VRChat's mic/OSC settings. Returns (note when it could not,
        the replaced values for undo). A running VRChat would overwrite them
        on exit, so only while it is closed."""
        from . import vrchat_settings
        if vrchat_settings.running():
            return self._tr("setup_vrchat_running"), {}
        try:
            previous = vrchat_settings.write(**changes)
        except OSError as e:
            log.warning("VRChat settings not written: %s", e)
            return self._tr("setup_vrchat_write_failed").format(error=e), {}
        if "mic" in changes:
            parts.append(self._tr("setup_changed_vrchat_mic").format(name=changes["mic"]))
        if "osc" in changes:
            parts.append(self._tr("setup_changed_vrchat_osc"))
        return "", dict(previous or {})

    def _undo_auto_setup(self) -> None:
        """Put back what the last auto setup changed, VRChat included."""
        undo = self._autofix_undo
        if not undo or self._device_applying or self._devices_reloading:
            return
        from . import vrchat_settings
        if undo["vrchat"]:
            if vrchat_settings.running():
                self._dashboard_note.setText(self._tr("setup_undo_vrchat_running"))
                return
            try:
                vrchat_settings.restore(undo["vrchat"])
            except OSError as e:
                log.warning("VRChat settings not restored: %s", e)
        self._autofix_undo = None
        self._setup_panel.set_undo_available(False)
        if not undo["cfg"]:
            self._dashboard_note.setText(self._tr("setup_undo_done"))
            self._sync_setup_panel(force=True)
            return

        def build():
            cfg = copy.deepcopy(self._controller.raw_cfg)
            for path, value in undo["cfg"].items():
                config_mod.set_path(cfg, path, value)
            cfg = config_mod.apply_app_profile(cfg)
            config_mod.save(cfg)
            return cfg

        def busy():
            self._device_applying = True
            self._set_dashboard_apply_enabled(False)

        thread = self._apply_config_async(
            build, fail_key="msg_device_failed", busy_key="msg_autofix_applying",
            note=self._dashboard_note, done_signal=self._signals.device_done,
            thread_name="vrclt-auto-setup-undo", on_busy=busy,
            on_build_error=self._sync_dashboard_devices)
        if thread is not None:
            self._device_thread = thread
            self._autofix_note = self._tr("setup_undo_done")

    # ---------------- VB-Cable install ----------------
    def _install_cable(self) -> None:
        if (self._cable_thread is not None and self._cable_thread.is_alive()) \
                or self._device_applying or self._devices_reloading:
            return
        box = QtWidgets.QMessageBox(self)
        box.setIcon(QtWidgets.QMessageBox.Icon.Information)
        box.setWindowTitle(self._tr("cable_install_title"))
        box.setText(self._tr("cable_install_confirm"))
        start = box.addButton(self._tr("cable_install_start"),
                              QtWidgets.QMessageBox.ButtonRole.AcceptRole)
        box.addButton(QtWidgets.QMessageBox.StandardButton.Cancel)
        box.exec()
        if box.clickedButton() is not start:
            return
        self._dashboard_note.setText(self._tr("msg_cable_downloading"))
        self._show_cable_progress(self._tr("msg_cable_downloading"))
        stages = {"download": "msg_cable_downloading", "verify": "msg_cable_verifying",
                  "install": "msg_cable_installing", "devices": "msg_cable_waiting"}

        def run():
            from . import cable_installer
            try:
                cable_installer.install(
                    lambda stage: self._signals.cable_progress.emit(stages[stage]))
            except cable_installer.InstallError as e:
                self._signals.cable_done.emit(e.kind, str(e))
                return
            except Exception as e:
                log.exception("VB-CABLE install failed")
                self._signals.cable_done.emit("failed", str(e))
                return
            self._signals.cable_done.emit("", "")

        self._cable_thread = threading.Thread(
            target=run, daemon=True, name="vrclt-cable-install")
        self._cable_thread.start()

    def _show_cable_progress(self, text: str) -> None:
        """A window-modal busy dialog for the whole install: the dashboard
        note alone was easy to miss. VB-Audio's own setup window is another
        process and stays usable on top of it."""
        dialog = self._cable_progress_dialog
        if dialog is None:
            dialog = QtWidgets.QProgressDialog(self)
            dialog.setWindowTitle(self._tr("cable_install_title"))
            label = QtWidgets.QLabel()
            label.setWordWrap(True)
            dialog.setLabel(label)
            dialog.setRange(0, 0)            # busy indicator
            dialog.setCancelButton(None)
            dialog.setMinimumDuration(0)
            dialog.setAutoClose(False)
            dialog.setAutoReset(False)
            dialog.setMinimumWidth(480)
            dialog.setWindowModality(QtCore.Qt.WindowModality.WindowModal)
            self._cable_progress_dialog = dialog
        dialog.setLabelText(text)
        dialog.show()

    def _close_cable_progress(self) -> None:
        dialog, self._cable_progress_dialog = self._cable_progress_dialog, None
        if dialog is not None:
            dialog.close()
            dialog.deleteLater()

    def _cable_notice(self, text: str, warning: bool = False) -> None:
        """Result of the install in its own window (non-blocking)."""
        box = QtWidgets.QMessageBox(self)
        box.setIcon(QtWidgets.QMessageBox.Icon.Warning if warning
                    else QtWidgets.QMessageBox.Icon.Information)
        box.setWindowTitle(self._tr("cable_install_title"))
        box.setText(text)
        box.setAttribute(QtCore.Qt.WidgetAttribute.WA_DeleteOnClose)
        box.open()
        self._cable_box = box

    def _cable_progress(self, key: str) -> None:
        self._dashboard_note.setText(self._tr(key))
        if self._cable_progress_dialog is not None:
            self._show_cable_progress(self._tr(key))

    def _cable_done(self, kind: str, detail: str) -> None:
        if kind == "cancelled":
            self._close_cable_progress()
            self._dashboard_note.setText(self._tr("msg_cable_cancelled"))
            return
        if kind:
            self._close_cable_progress()
            text = self._tr("msg_cable_failed").format(error=detail[:160])
            self._dashboard_note.setText(text)
            self._cable_notice(text, warning=True)
            return
        # PortAudio only sees a new device after a reinit; auto setup follows
        self._dashboard_note.setText(self._tr("msg_cable_done"))
        self._show_cable_progress(self._tr("msg_cable_done"))
        self._cable_followup = True
        self._reload_devices()

    def _find_mic(self) -> None:
        if self._device_applying or self._devices_reloading:
            return
        from .ui.mic_finder import MicFinderDialog
        env = self._setup_env
        wanted = str(self._controller.cfg.get("outbound", {}).get("mic_device", "") or "")
        current = setup_check.find_device(self._inputs, wanted) if wanted \
            else (env.default_input if env is not None else "")
        dialog = MicFinderDialog(
            self._tr, setup_check.real_inputs(self._inputs), current=current,
            threshold=float(self._controller.cfg.get("audio", {}).get(
                "voice_rms_threshold", 90.0)),
            parent=self)
        if dialog.exec() != QtWidgets.QDialog.DialogCode.Accepted or not dialog.selected:
            return
        if dialog.selected == current and wanted:
            return
        self._mic_device.setCurrentText(dialog.selected)
        self._apply_dashboard_device("outbound.mic_device", self._mic_device)

    def _apply_app_mode(self, mode: str) -> None:
        mode = (mode or "").strip()
        current = self._controller.cfg.get("app", {}).get("mode", "vrchat")
        if not mode or self._app_mode_applying:
            self._set_app_mode_checked(current)
            return

        def build():
            cfg = copy.deepcopy(self._controller.raw_cfg)
            cfg.setdefault("app", {})["mode"] = mode
            cfg = config_mod.apply_app_profile(cfg, force=True)
            if mode == current and self._profile_runtime_snapshot(cfg) == \
                    self._profile_runtime_snapshot(self._controller.cfg):
                # clicking the already-selected mode: nothing to write or restart
                self._set_app_mode_checked(current)
                return None
            config_mod.save(cfg)
            return cfg

        def busy():
            self._app_mode_applying = True
            self._set_dashboard_apply_enabled(False)

        thread = self._apply_config_async(
            build, fail_key="msg_mode_failed", busy_key="msg_mode_applying",
            note=self._dashboard_note, done_signal=self._signals.mode_done,
            thread_name="vrclt-mode-restart", on_busy=busy,
            on_build_error=lambda: self._set_app_mode_checked(current))
        if thread is not None:
            self._mode_thread = thread

    def _mode_done(self, ok: bool) -> None:
        self._app_mode_applying = False
        self._set_dashboard_apply_enabled(True)
        self._set_app_mode_checked(self._controller.cfg.get("app", {}).get("mode", "vrchat"))
        self._sync_text_only()
        self._dashboard_note.setText(
            self._mode_applied_note() if ok else self._tr("msg_saved_start_failed"))
        self._sync_hotkeys(force=True)
        self._populate_settings()

    def _mode_applied_note(self) -> str:
        """Custom mode without a target picked yet keeps capturing whatever it
        was - say so instead of a bare "applied" the user can't act on."""
        cfg = self._controller.cfg
        if cfg.get("app", {}).get("mode") == "custom" and not _get_path(
                cfg, "app.profiles.custom.process"):
            return self._tr("msg_mode_custom_unset").format(
                process=_get_path(cfg, "inbound.process") or "?")
        return self._tr("msg_mode_applied")

    def _set_dashboard_apply_enabled(self, enabled: bool) -> None:
        enabled = enabled and not self._speaker_context_applying
        for btn in self._app_mode_buttons.values():
            btn.setEnabled(enabled)
        self._text_only.setEnabled(enabled)
        self._keep_speaker_context.setEnabled(enabled)
        self._out_engine.setEnabled(enabled)
        self._in_engine.setEnabled(enabled)
        if hasattr(self, "_mic_device"):
            self._mic_device.setEnabled(enabled)
        if hasattr(self, "_voice_out_device"):
            self._voice_out_device.setEnabled(enabled)
        if hasattr(self, "_btn_test_out"):
            self._btn_test_out.setEnabled(enabled)
        if hasattr(self, "_btn_find_mic"):
            self._btn_find_mic.setEnabled(enabled)
        if hasattr(self, "_easy_mic_device"):
            self._easy_mic_device.setEnabled(enabled)
            self._btn_easy_find_mic.setEnabled(enabled)

    def _set_app_mode_checked(self, mode: str) -> None:
        for key, btn in self._app_mode_buttons.items():
            btn.setChecked(key == mode)

    @staticmethod
    def _profile_runtime_snapshot(cfg: dict) -> tuple:
        paths = (
            "inbound.process",
            "ui.mode",
            "outbound.voice_output",
            "outbound.passthrough_while_translating",
            "outbound.chatbox",
            "control.enabled",
            "overlay.enabled",
            "wrist_ui.enabled",
        )
        return tuple(_get_path(cfg, path) for path in paths)

    def _apply_text_only(self, enabled: bool) -> None:
        if self._app_mode_applying:
            self._sync_text_only()
            return

        def build():
            cfg = copy.deepcopy(self._controller.raw_cfg)
            if enabled:
                cfg.setdefault("app", {})["mode"] = "vrchat"
            cfg.setdefault("outbound", {})["text_only"] = bool(enabled)
            cfg = config_mod.apply_app_profile(cfg, force=True)
            config_mod.save(cfg)
            return cfg

        def busy():
            self._app_mode_applying = True
            self._set_dashboard_apply_enabled(False)

        thread = self._apply_config_async(
            build, fail_key="msg_text_only_failed", busy_key="msg_text_only_applying",
            note=self._dashboard_note, done_signal=self._signals.mode_done,
            thread_name="vrclt-text-only-restart", on_busy=busy,
            on_build_error=self._sync_text_only)
        if thread is not None:
            self._mode_thread = thread

    def _fill_engine_pickers(self) -> None:
        for combo, follows in ((self._out_engine, False), (self._in_engine, True)):
            blocked = combo.blockSignals(True)
            try:
                combo.clear()
                if follows:
                    combo.addItem(self._tr("engine_same_as_translation"), "")
                for prov in config_mod.PROVIDERS:
                    combo.addItem(ENGINE_NAMES[prov], prov)
            finally:
                combo.blockSignals(blocked)
        self._sync_engine_pickers()

    def _sync_engine_pickers(self) -> None:
        if not hasattr(self, "_in_engine"):
            return
        cfg = self._controller.cfg
        chosen = str(cfg.get("inbound", {}).get("provider") or "").strip().lower()
        for combo, value in ((self._out_engine, config_mod.provider(cfg)),
                             (self._in_engine, chosen if chosen in config_mod.PROVIDERS else "")):
            index = combo.findData(value)
            if index >= 0 and index != combo.currentIndex() and not combo.view().isVisible():
                blocked = combo.blockSignals(True)
                try:
                    combo.setCurrentIndex(index)
                finally:
                    combo.blockSignals(blocked)

    def _apply_engines(self) -> None:
        """Dashboard engine pickers (Full view): save and restart, but only
        when every engine the runtime would start has a usable key."""
        if self._app_mode_applying:
            self._sync_engine_pickers()
            return
        out_prov = self._out_engine.currentData() or "gemini"
        in_prov = self._in_engine.currentData() or ""
        current = self._controller.raw_cfg
        if out_prov == config_mod.provider(current) and in_prov == str(
                current.get("inbound", {}).get("provider") or "").strip().lower():
            return

        def build():
            cfg = copy.deepcopy(self._controller.raw_cfg)
            cfg["provider"] = out_prov
            cfg.setdefault("inbound", {})["provider"] = in_prov
            for prov in config_mod.providers_in_use(cfg):
                key = config_mod.api_key_for(cfg, prov)
                if not key:
                    raise ValueError(self._tr("msg_engine_key_missing").format(
                        engine=ENGINE_NAMES[prov]))
                error = config_mod.api_key_validation_error(
                    key, provider_label="DashScope" if prov == "qwen" else ENGINE_NAMES[prov])
                if error:
                    raise ValueError(error)
            qw = cfg.get("qwen", {})
            if "qwen" in config_mod.providers_in_use(cfg)                     and str(qw.get("endpoint", "intl") or "intl").strip() != "beijing"                     and not str(qw.get("workspace_id", "") or "").strip()                     and not str(qw.get("base_url", "") or "").strip():
                raise ValueError(self._tr("err_qwen_workspace_required"))
            config_mod.save(cfg)
            return cfg

        def busy():
            self._app_mode_applying = True
            self._set_dashboard_apply_enabled(False)

        thread = self._apply_config_async(
            build, fail_key="msg_engine_failed", busy_key="msg_engine_applying",
            note=self._dashboard_note, done_signal=self._signals.engine_done,
            thread_name="vrclt-engine-restart", on_busy=busy,
            on_build_error=self._sync_engine_pickers)
        if thread is not None:
            self._mode_thread = thread

    def _engine_done(self, ok: bool) -> None:
        self._app_mode_applying = False
        self._set_dashboard_apply_enabled(True)
        self._sync_engine_pickers()
        self._sync_text_only()
        self._dashboard_note.setText(
            self._tr("msg_engine_applied") if ok else self._tr("msg_saved_start_failed"))
        self._populate_settings()
        self._sync_setup_panel(force=True)

    def _sync_text_only(self) -> None:
        blocked = self._text_only.blockSignals(True)
        try:
            self._text_only.setChecked(config_mod.is_text_only(self._controller.cfg))
            self._text_only.setEnabled(
                not self._app_mode_applying
                and not self._speaker_context_applying
                and self._controller.cfg.get("app", {}).get("mode", "vrchat") == "vrchat")
        finally:
            self._text_only.blockSignals(blocked)

    def _sync_speaker_context(self) -> None:
        sx = self._controller.cfg.get("soniox", {})
        signature = (sx.get("keep_speaker_context", True),
                     sx.get("speaker_context_idle_sec", 60.0))
        previous = getattr(self, "_speaker_context_config_signature", None)
        self._speaker_context_config_signature = signature
        if previous is not None and signature != previous:
            # VR can change this setting without replacing unrelated draft
            # values the user is editing on the desktop Settings page.
            paths = ("soniox.keep_speaker_context", "soniox.speaker_context_idle_sec")
            self._settings_form.sync_from_config(tuple(
                path for path, old, new in zip(paths, previous, signature) if old != new))
        soniox_full = ("soniox" in config_mod.providers_in_use(self._controller.cfg)
                       and not self._easy_mode())
        self._keep_speaker_context.setVisible(soniox_full)
        self._soniox_idle_hint.setVisible(soniox_full)
        seconds = config_mod.soniox_idle_disconnect_sec(self._controller.cfg)
        self._soniox_idle_hint.setToolTip(self._tr(
            "f.soniox.speaker_context_idle_sec.tip" if _get_path(
                self._controller.cfg, "soniox.keep_speaker_context", True)
            else "f.audio.mic_idle_disconnect_sec.tip"))
        self._soniox_idle_hint.setText(
            self._tr("soniox_idle_timeout_status").format(seconds=f"{seconds:g}")
            if seconds > 0 else self._tr("soniox_idle_timeout_disabled"))
        if not self._speaker_context_applying:
            blocked = self._keep_speaker_context.blockSignals(True)
            try:
                self._keep_speaker_context.setChecked(bool(_get_path(
                    self._controller.cfg, "soniox.keep_speaker_context", True)))
            finally:
                self._keep_speaker_context.blockSignals(blocked)
        self._keep_speaker_context.setEnabled(
            not self._speaker_context_applying and not self._app_mode_applying
            and not self._device_applying and not self._devices_reloading
            and (self._test_thread is None or not self._test_thread.is_alive())
            and self._btn_save.isEnabled())

    def _apply_speaker_context(self, enabled: bool) -> None:
        if (self._speaker_context_applying or self._app_mode_applying
                or self._device_applying or self._devices_reloading
                or (self._test_thread is not None and self._test_thread.is_alive())
                or not self._btn_save.isEnabled()
                or "soniox" not in config_mod.providers_in_use(self._controller.cfg)):
            self._sync_speaker_context()
            return
        if enabled == bool(_get_path(
                self._controller.cfg, "soniox.keep_speaker_context", True)):
            return

        def build():
            cfg = copy.deepcopy(self._controller.raw_cfg)
            _set_path(cfg, "soniox.keep_speaker_context", bool(enabled))
            config_mod.save(cfg)
            return cfg

        def busy():
            self._speaker_context_applying = True
            self._set_dashboard_apply_enabled(False)
            self._btn_save.setEnabled(False)
            self._btn_reset_config.setEnabled(False)
            self._btn_devices.setEnabled(False)

        thread = self._apply_config_async(
            build, fail_key="msg_save_failed", busy_key="msg_save_restarting",
            note=self._dashboard_note, done_signal=self._signals.speaker_context_done,
            thread_name="vrclt-speaker-context-restart", on_busy=busy,
            on_build_error=self._sync_speaker_context)
        if thread is not None:
            self._speaker_context_thread = thread

    def _speaker_context_done(self, ok: bool) -> None:
        self._speaker_context_applying = False
        self._set_dashboard_apply_enabled(True)
        self._btn_save.setEnabled(True)
        self._btn_reset_config.setEnabled(True)
        self._btn_devices.setEnabled(True)
        self._sync_speaker_context()
        # Update just this setting, preserving other unsaved form edits.
        self._settings_form.sync_from_config(("soniox.keep_speaker_context",))
        self._dashboard_note.setText(
            self._tr("msg_applied") if ok else self._tr("msg_saved_start_failed"))

    def _set_overlay_font_size(self, value: int) -> None:
        self._controller.set_overlay_font_size(value)
        self._desktop_overlay.refresh()

    def _toggle_overlay_move(self) -> None:
        st = self._controller.state
        st.edit_mode = not st.edit_mode
        if st.edit_mode:
            self._desktop_overlay.show_for_edit()

    def _reset_overlay_position(self) -> None:
        self._desktop_overlay.reset_position()
        self._controller.state.request_position_reset()

    def _toggle_translation(self) -> None:
        self._controller.set_translation_on(not self._controller.state.translation_on)

    def _toggle_subtitles(self) -> None:
        self._controller.set_subtitles_on(not self._controller.state.subtitles_on)

    def _sync_hotkeys(self, force: bool = False) -> None:
        cfg = self._controller.cfg.get("hotkeys", {})
        enabled = bool(cfg.get("enabled", True))
        enabled_in_vr = bool(cfg.get("enabled_in_vr", True))
        translation = str(cfg.get("translation_toggle", "") or "")
        subtitles = str(cfg.get("subtitles_toggle", "") or "")
        hold = str(cfg.get("translation_hold", "") or "")
        pc_mode = resolve_ui_mode(self._controller.cfg) == "desktop"
        active = enabled and (pc_mode or enabled_in_vr)
        signature = (active, translation, subtitles, hold)
        if not force and signature == self._hotkey_signature:
            return
        self._hotkey_signature = signature
        if not active:
            self._hotkeys.configure([])
            return
        self._hotkeys.configure([
            HotkeyRegistration(
                HOTKEY_TRANSLATION_ID, "translation toggle", translation,
                self._signals.translation_hotkey.emit),
            HotkeyRegistration(
                HOTKEY_SUBTITLES_ID, "subtitles toggle", subtitles,
                self._signals.subtitles_hotkey.emit),
            HotkeyRegistration(
                HOTKEY_TRANSLATION_HOLD_ID, "translation hold", hold,
                lambda: self._signals.translation_hold.emit(True),
                on_release=lambda: self._signals.translation_hold.emit(False)),
        ])

    def _add_output_language_from_input(self) -> None:
        self._add_language_from_input(
            self._out_lang_add,
            self._controller.cfg.get("control", {}).get("languages", []),
            self._controller.add_output_language,
        )

    def _add_inbound_language_from_input(self) -> None:
        self._add_language_from_input(
            self._sub_lang_add,
            self._controller.cfg.get("inbound", {}).get("languages", []),
            self._controller.add_inbound_language,
        )

    def _add_language_from_input(self, edit: QtWidgets.QComboBox, existing: list[str],
                                 add_fn) -> None:
        code = code_from_language_combo(edit, existing)
        if not code:
            return
        add_fn(code)
        edit.setCurrentIndex(-1)
        edit.setEditText("")
        self._dashboard_note.setText(self._tr("msg_applied"))

    def _pick_out_lang(self, label: str) -> None:
        code = self._code_for_label(label, self._controller.cfg.get("control", {}).get("languages", []))
        if code:
            self._controller.set_target_language(code)

    def _pick_sub_lang(self, label: str) -> None:
        code = self._code_for_label(label, self._controller.cfg.get("inbound", {}).get("languages", []))
        if code:
            self._controller.set_inbound_language(code)

    def _pick_src_lang(self) -> None:
        self._controller.set_source_language(
            code_from_language_combo(self._src_lang, []))

    def _pick_in_src_lang(self) -> None:
        self._controller.set_inbound_source_language(
            code_from_language_combo(self._in_src_lang, []))

    def _pick_ui_lang(self, label: str) -> None:
        for code, text in i18n.UI_LANG_LABELS.items():
            if text == label:
                self._controller.set_ui_lang(code)
                break

    def _pick_close_action(self) -> None:
        code = self._close_action.currentData()
        if code:
            self._controller.set_close_action(str(code))

    def _sync_close_action(self) -> None:
        blocked = self._close_action.blockSignals(True)
        try:
            current = self._controller.close_action()
            self._close_action.clear()
            for code in config_mod.CLOSE_ACTIONS:
                self._close_action.addItem(self._tr(f"close_action_{code}"), code)
            idx = self._close_action.findData(current)
            self._close_action.setCurrentIndex(idx if idx >= 0 else 0)
        finally:
            self._close_action.blockSignals(blocked)

    @staticmethod
    def _code_for_label(label: str, codes: list[str]) -> str:
        return language_code_from_text(label, codes)

    # ---------------- refresh ----------------
    def _queue_refresh(self) -> None:
        # A burst of streaming tokens can notify several subscribers at once.
        # Render the latest state once per frame, keeping the regular timer as
        # a fallback for changes that don't emit controller notifications.
        if not self._refresh_pending.isActive():
            self._refresh_pending.start(40 if self.isVisible() else 1000)

    def _refresh(self) -> None:
        self._refresh_pending.stop()
        st = self._controller.state
        if st.ui_lang != self._last_ui_lang:
            self._last_ui_lang = st.ui_lang
            _apply_app_font(QtWidgets.QApplication.instance(), st.ui_lang)
            self._apply_i18n()
            self._populate_settings()
            self._setup_checked_at = 0.0
        if self._view_buttons[True].isChecked() != self._easy_mode() \
                or not any(b.isChecked() for b in self._view_buttons.values()):
            self._apply_view_mode()
        revision = getattr(self._controller, "config_revision", 0)
        if revision != self._last_config_revision:
            self._last_config_revision = revision
            self._sync_settings_from_config()
        # unconditional: cheap signature early-out, and it un-stales the
        # ui.mode=auto gate when SteamVR starts/stops between config saves
        self._sync_hotkeys()
        connected, status_key, detail = self._controller.get_status_info()
        if connected:
            dot_state = "ok"
        elif status_key in ("status_api_key_invalid", "status_api_key_required",
                            "status_failed"):
            dot_state = "err"
        elif status_key in ("status_running", "status_degraded",
                            "status_reconnecting", "status_quota_exceeded"):
            dot_state = "warn"
        else:
            dot_state = "idle"
        self._set_state_prop(self._status_dot, "state", dot_state)
        conn_key = "conn_on" if connected else "conn_off"
        self._status_text.setText(
            f"{i18n.tr(st.ui_lang, status_key)} | {i18n.tr(st.ui_lang, conn_key)}")
        self._error_text.setText(self._error_label(detail))
        self._error_text.setVisible(bool(detail))

        cfg = self._controller.cfg
        out_prov = config_mod.provider(cfg)
        in_prov = config_mod.inbound_provider(cfg)
        # the live preview shows the subtitles: the subtitle engine's labels
        self._speaker_badge.setVisible(config_mod.has_speaker_ids(cfg, "inbound"))
        self._sync_speaker_context()
        names = {"gemini": "Gemini Live", "qwen": "Qwen Live",
                 "openai": "OpenAI", "soniox": "Soniox"}
        self._engine_button.setText(
            names.get(out_prov, out_prov) if in_prov == out_prov
            else f"{names.get(out_prov, out_prov)} / {names.get(in_prov, in_prov)}")
        self._sync_engine_pickers()
        self._sync_setup_panel()

        self._btn_trans.setText(i18n.tr(st.ui_lang, "btn_trans_on" if st.translation_on else "btn_trans_off"))
        self._set_state_prop(self._btn_trans, "on",
                             "true" if st.translation_on else "false")
        self._btn_sub.setText(i18n.tr(st.ui_lang, "btn_sub_on" if st.subtitles_on else "btn_sub_off"))
        self._set_state_prop(self._btn_sub, "on",
                             "true" if st.subtitles_on else "false")
        self._btn_overlay_move.setText(
            i18n.tr(st.ui_lang, "btn_overlay_done" if st.edit_mode else "btn_overlay_move"))
        self._set_state_prop(self._btn_overlay_move, "active",
                             "true" if st.edit_mode else "false")

        if not self._app_mode_applying:
            self._set_app_mode_checked(self._controller.cfg.get("app", {}).get("mode", "vrchat"))
            self._sync_text_only()

        blocked = self._overlay_font_size.blockSignals(True)
        try:
            self._overlay_font_size.setValue(
                int(self._controller.cfg.get("overlay", {}).get("font_size", 27)))
        finally:
            self._overlay_font_size.blockSignals(blocked)

        if not self._tts_gain_slider.isSliderDown() and \
                not self._tts_gain_timer.isActive():
            blocked = self._tts_gain_slider.blockSignals(True)
            try:
                self._tts_gain_slider.setValue(
                    round(self._controller.tts_gain() * 100))
                self._tts_gain_label.setText(f"{self._tts_gain_slider.value()}%")
            finally:
                self._tts_gain_slider.blockSignals(blocked)
        want_meter = self.isVisible() and \
            self._tabs.currentIndex() == self._tab_dashboard_idx
        if want_meter != self._meter_timer.isActive():
            (self._meter_timer.start if want_meter else self._meter_timer.stop)()
        self._log_panel.set_active(
            self.isVisible() and self._tabs.currentIndex() == self._tab_logs_idx)

        self._sync_combo(self._out_lang, [
            language_label(c) for c in self._controller.cfg.get("control", {}).get("languages", ["en"])
        ], language_label(st.target_language))
        self._sync_combo(self._sub_lang, [
            language_label(c) for c in self._controller.cfg.get("inbound", {}).get("languages", ["ko"])
        ], language_label(st.inbound_language))
        self._sync_source_pickers()
        self._sync_combo(self._ui_lang, [i18n.UI_LANG_LABELS[c] for c in i18n.LANGS],
                         i18n.UI_LANG_LABELS.get(st.ui_lang, st.ui_lang))
        self._sync_dashboard_devices()
        if self.isVisible() and self._tabs.currentIndex() == self._tab_settings_idx:
            self._sync_steamvr_autolaunch()
        self._dashboard_note.setVisible(bool(self._dashboard_note.text()))
        self._layout_dashboard_rows()

        from .ui.subtitle_text import subtitle_snapshot, subtitle_html
        finals, partial = subtitle_snapshot(self._controller)
        snapshot = (tuple(finals), partial, st.ui_lang)
        if snapshot != self._subtitle_snapshot:
            self._subtitle_snapshot = snapshot
            markup = subtitle_html(finals, partial, self._tr)
            if markup:
                self._subtitle_view.setHtml(markup)
                bar = self._subtitle_view.verticalScrollBar()
                bar.setValue(bar.maximum())
            else:
                self._subtitle_view.clear()  # keeps the placeholder visible

    @staticmethod
    def _set_state_prop(widget: QtWidgets.QWidget, name: str, value: str) -> None:
        # dynamic QSS properties need an explicit re-polish to restyle; the
        # 250 ms refresh timer calls this constantly, so diff first
        if widget.property(name) != value:
            widget.setProperty(name, value)
            widget.style().unpolish(widget)
            widget.style().polish(widget)

    @staticmethod
    def _sync_combo(combo: QtWidgets.QComboBox, items: list[str], current: str) -> None:
        blocked = combo.blockSignals(True)
        try:
            existing = [combo.itemText(i) for i in range(combo.count())]
            if existing != items:
                combo.clear()
                combo.addItems(items)
            combo.setCurrentText(current)
        finally:
            combo.blockSignals(blocked)

    def _sync_dashboard_devices(self, force: bool = False) -> None:
        if not hasattr(self, "_mic_device") or not hasattr(self, "_voice_out_device"):
            return
        if self._device_applying and not force:
            return
        focus = QtWidgets.QApplication.focusWidget()
        mic_active = focus is not None and any(
            focus is combo or combo.isAncestorOf(focus)
            for combo in (self._mic_device, self._easy_mic_device))
        out_active = focus is not None and (
            focus is self._voice_out_device or self._voice_out_device.isAncestorOf(focus))
        if force or not mic_active:
            for combo in (self._mic_device, self._easy_mic_device):
                self._sync_device_combo(
                    combo, self._mic_choices(),
                    self._controller.cfg.get("outbound", {}).get("mic_device", ""))
        if force or not out_active:
            self._sync_device_combo(
                self._voice_out_device,
                self._outputs,
                self._controller.cfg.get("outbound", {}).get("tts_device", ""))

    @staticmethod
    def _sync_device_combo(combo: QtWidgets.QComboBox, items: list[str], current: str) -> None:
        current = "" if current is None else str(current)
        values = list(items or [""])
        if "" not in values:
            values.insert(0, "")
        if current and current not in values:
            values.append(current)
        blocked = combo.blockSignals(True)
        try:
            existing = [combo.itemText(i) for i in range(combo.count())]
            if existing != values:
                combo.clear()
                combo.addItems(values)
            combo.setCurrentText(current)
        finally:
            combo.blockSignals(blocked)

    # ---------------- window/tray lifecycle ----------------
    def _layout_dashboard_rows(self) -> None:
        """Use full rows when translated labels no longer fit side by side."""
        if not hasattr(self, "_dashboard_scroll"):
            return
        page = self._dashboard_scroll.widget()
        margins = page.layout().contentsMargins()
        available = (self._dashboard_scroll.viewport().width()
                     - margins.left() - margins.right())
        changed = self._setup_panel.fit_header(available)
        # the cards' own rows first, then whether the two option cards still
        # fit side by side
        for layout in (self._pipe_layout, self._audio_layout, self._context_layout,
                       self._display_layout, self._app_layout, self._options_layout):
            widths = []
            for index in range(layout.count()):
                widget = layout.itemAt(index).widget()
                if widget is not None and not widget.isHidden():
                    widths.append(max(widget.minimumWidth(), widget.minimumSizeHint().width()))
            padding = layout.contentsMargins()
            required = (sum(widths) + max(0, len(widths) - 1) * layout.spacing()
                        + padding.left() + padding.right())
            # plus every frame and padding between the row and the page: the
            # audio row sits in a widget inside its card, and counting only
            # that inner widget left the full view 22 px too wide at 780 px
            holder = layout.parentWidget()
            while holder is not None and holder is not page:
                frame = holder.contentsMargins()
                required += frame.left() + frame.right()
                if holder.layout() is not None and holder.layout() is not layout:
                    inner = holder.layout().contentsMargins()
                    required += inner.left() + inner.right()
                holder = holder.parentWidget()
            stacked = required > available or (layout is self._pipe_layout and self.width() < 900)
            direction = (QtWidgets.QBoxLayout.Direction.TopToBottom if stacked
                         else QtWidgets.QBoxLayout.Direction.LeftToRight)
            if layout.direction() != direction:
                layout.setDirection(direction)
                layout.activate()
                changed = True
            if layout is self._context_layout:
                self._soniox_idle_hint.setAlignment(
                    QtCore.Qt.AlignmentFlag.AlignVCenter |
                    (QtCore.Qt.AlignmentFlag.AlignLeft if stacked else QtCore.Qt.AlignmentFlag.AlignRight))
        if changed:
            page.layout().invalidate()
            page.layout().activate()

    def resizeEvent(self, event) -> None:
        super().resizeEvent(event)
        self._layout_dashboard_rows()

    def showEvent(self, event) -> None:
        super().showEvent(event)
        if hasattr(self, "_timer"):
            self._timer.setInterval(250)
            self._queue_refresh()

    def hideEvent(self, event) -> None:
        super().hideEvent(event)
        if hasattr(self, "_timer"):
            self._timer.setInterval(1000)
            self._meter_timer.stop()
            self._log_panel.set_active(False)

    def _show_main(self) -> None:
        self.showNormal()
        self.raise_()
        self.activateWindow()

    def _show_settings(self) -> None:
        self._tabs.setCurrentIndex(1)
        self._show_main()

    def closeEvent(self, event) -> None:
        if self._quitting:
            event.accept()
        elif self._controller.close_action() == "exit":
            event.accept()
            self._quit()
        else:
            event.ignore()
            self.hide()
            self._tray.show_message("vrclt", self._tr("tray_still_running"), 1500)

    def _quit(self) -> None:
        self._quitting = True
        self._hotkeys.stop()
        self._desktop_overlay.close()
        self._tray.hide()
        # shutdown (not stop): a restart queued behind the lifecycle lock
        # must not resurrect the runtime during interpreter teardown
        self._controller.shutdown()
        QtWidgets.QApplication.quit()


def run_qt_app(controller, log_file: Path) -> int:
    app = QtWidgets.QApplication([])
    app.setApplicationName("vrclt")
    app.setQuitOnLastWindowClosed(False)
    # own AppUserModelID so the taskbar shows our icon, not python.exe's,
    # on dev runs (frozen exes get it from the embedded .ico anyway)
    try:
        import ctypes
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID("shgeum.vrclt")
    except Exception:
        pass
    icon_file = icon_path()
    if icon_file.exists():
        app.setWindowIcon(QtGui.QIcon(str(icon_file)))
    _install_app_font(app, controller.state.ui_lang)
    win = MainWindow(controller, log_file)
    win.show()
    threading.Thread(target=controller.start, daemon=True, name="vrclt-start").start()
    app.aboutToQuit.connect(controller.shutdown)
    return app.exec()
