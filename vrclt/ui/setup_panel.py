"""Dashboard setup checklist: what is ready, what is not, and how to fix it.

Rows come from setup_check.run_checks(). The panel only renders them; the
window gathers the environment, applies [Auto setup] and opens the mic finder.
It expands by itself whenever the set of problems changes and folds down to a
one-line summary once everything is ready.
"""
import logging

from PySide6 import QtCore, QtWidgets

from .. import platform_support, setup_check
from .widgets import WrapLabel

log = logging.getLogger(__name__)

KEY_URLS = {
    "gemini": "https://aistudio.google.com/apikey",
    "qwen": "https://modelstudio.console.alibabacloud.com/?tab=model#/api-key",
    "qwen_beijing": "https://bailian.console.aliyun.com/?tab=model#/api-key",
    "openai": "https://platform.openai.com/api-keys",
    "soniox": "https://console.soniox.com/",
}
API_KEY_FIELDS = {"gemini": "api_key", "qwen": "qwen.api_key",
                  "openai": "openai.api_key", "soniox": "soniox.api_key"}
PROVIDER_LABELS = {"gemini": "Gemini", "qwen": "Qwen (DashScope)",
                   "openai": "OpenAI", "soniox": "Soniox"}
ICONS = {setup_check.OK: "✓", setup_check.INFO: "i",
         setup_check.WARN: "!", setup_check.ERROR: "×"}


class SetupPanel(QtWidgets.QFrame):
    def __init__(self, tr, *, on_open_settings, on_open_url, on_auto_fix,
                 on_find_mic, on_recheck, on_install_cable=None, on_undo=None):
        super().__init__()
        self._on_install_cable = on_install_cable
        self._on_undo = on_undo
        self._undo_available = False
        self._tr = tr
        self._on_open_settings = on_open_settings
        self._on_open_url = on_open_url
        self._on_auto_fix = on_auto_fix
        self._on_find_mic = on_find_mic
        self._on_recheck = on_recheck
        self._items: list = []
        self._context = {"provider": "gemini", "endpoint": "", "app": "VRChat"}
        self._last_sync = None
        self._issues = None
        self._expanded = False

        self.setObjectName("setupPanel")
        layout = QtWidgets.QVBoxLayout(self)
        layout.setContentsMargins(14, 10, 14, 10)
        layout.setSpacing(8)
        # title and buttons share a row, or stack when the window is narrow
        # (see fit_header); the wrappers keep each group on one line
        header = QtWidgets.QBoxLayout(QtWidgets.QBoxLayout.Direction.LeftToRight)
        header.setSpacing(10)
        self._header = header
        self._title_wrap = QtWidgets.QWidget()
        self._title_wrap.setObjectName("setupHeader")
        title_lay = QtWidgets.QHBoxLayout(self._title_wrap)
        title_lay.setContentsMargins(0, 0, 0, 0)
        title_lay.setSpacing(10)
        self._buttons_wrap = QtWidgets.QWidget()
        self._buttons_wrap.setObjectName("setupHeader")
        buttons_lay = QtWidgets.QHBoxLayout(self._buttons_wrap)
        buttons_lay.setContentsMargins(0, 0, 0, 0)
        buttons_lay.setSpacing(10)
        self._icon = self._badge()
        self._title = QtWidgets.QLabel()
        self._title.setObjectName("setupTitle")
        self._summary = WrapLabel()
        self._summary.setObjectName("setupText")
        self._summary.setAlignment(QtCore.Qt.AlignmentFlag.AlignLeft
                                   | QtCore.Qt.AlignmentFlag.AlignVCenter)
        self._btn_fix = QtWidgets.QPushButton()
        self._btn_fix.setObjectName("primaryButton")
        self._btn_fix.clicked.connect(lambda: self._on_auto_fix())
        self._btn_undo = QtWidgets.QPushButton()
        self._btn_undo.clicked.connect(lambda: self._on_undo and self._on_undo())
        self._btn_recheck = QtWidgets.QPushButton()
        self._btn_recheck.clicked.connect(lambda: self._on_recheck())
        self._btn_toggle = QtWidgets.QPushButton()
        self._btn_toggle.clicked.connect(self._toggle)
        title_lay.addWidget(self._icon)
        title_lay.addWidget(self._title)
        title_lay.addWidget(self._summary, 1)
        buttons_lay.addWidget(self._btn_fix)
        buttons_lay.addWidget(self._btn_undo)
        buttons_lay.addWidget(self._btn_recheck)
        buttons_lay.addWidget(self._btn_toggle)
        buttons_lay.addStretch(1)
        header.addWidget(self._title_wrap, 1)
        header.addWidget(self._buttons_wrap)
        layout.addLayout(header)
        self._body = QtWidgets.QWidget()
        self._body.setObjectName("setupBody")
        self._grid = QtWidgets.QGridLayout(self._body)
        self._grid.setContentsMargins(0, 4, 0, 0)
        self._grid.setHorizontalSpacing(12)
        self._grid.setVerticalSpacing(8)
        self._grid.setColumnStretch(2, 1)
        layout.addWidget(self._body)
        self._render()

    # ------------------------------------------------------------ state
    def sync(self, items, *, provider: str = "gemini", endpoint: str = "",
             app: str = "VRChat") -> None:
        """Diffed: called from the refresh tick, repaints only on change."""
        signature = (tuple(items), provider, endpoint, app)
        if signature == self._last_sync:
            return
        self._last_sync = signature
        self._items = list(items)
        self._context = {"provider": provider, "endpoint": endpoint, "app": app}
        issues = tuple(item.key for item in self._items if item.status != setup_check.OK)
        if issues != self._issues:
            self._issues = issues
            self._expanded = bool(issues)
        self._render()

    def fit_header(self, available: int) -> bool:
        """Move the buttons under the title when one row would be wider than
        the space the dashboard has; returns True when the layout changed."""
        margins = self.layout().contentsMargins()
        need = (self._title_wrap.minimumSizeHint().width()
                + self._buttons_wrap.sizeHint().width() + self._header.spacing()
                + margins.left() + margins.right())
        direction = (QtWidgets.QBoxLayout.Direction.TopToBottom if need > available
                     else QtWidgets.QBoxLayout.Direction.LeftToRight)
        if self._header.direction() == direction:
            return False
        self._header.setDirection(direction)
        return True

    def set_undo_available(self, available: bool) -> None:
        self._undo_available = bool(available)
        self._render()

    def issue_count(self) -> int:
        return sum(1 for item in self._items
                   if item.status in (setup_check.WARN, setup_check.ERROR))

    def _toggle(self) -> None:
        self._expanded = not self._expanded
        self._render()

    def retranslate(self) -> None:
        self._render()

    # ------------------------------------------------------------ rendering
    def _render(self) -> None:
        status = setup_check.worst(self._items)
        issues = self.issue_count()
        self._set_status(self._icon, status)
        self.setProperty("status", status)
        self.style().unpolish(self)
        self.style().polish(self)
        self._title.setText(self._tr("setup_panel_title"))
        self._summary.setText(
            self._tr("setup_summary_issues").format(n=issues) if issues
            else self._tr("setup_summary_ok"))
        self._btn_fix.setText(self._tr("setup_btn_auto_fix"))
        self._btn_fix.setVisible(any(item.fixable for item in self._items))
        self._btn_undo.setText(self._tr("setup_btn_undo"))
        self._btn_undo.setVisible(self._undo_available and self._on_undo is not None)
        self._btn_recheck.setText(self._tr("setup_btn_recheck"))
        self._btn_toggle.setText(self._tr(
            "setup_btn_hide" if self._expanded else "setup_btn_show"))
        self._body.setVisible(self._expanded)
        while self._grid.count():
            widget = self._grid.takeAt(0).widget()
            if widget is not None:
                widget.hide()   # deleteLater waits for the event loop
                widget.deleteLater()
        for row, item in enumerate(self._items):
            icon = self._badge()
            self._set_status(icon, item.status)
            title = WrapLabel(self._tr(f"setup_{item.key}"))
            title.setObjectName("setupItemTitle")
            title.setFixedWidth(150)
            detail = WrapLabel(self.detail_text(item))
            detail.setObjectName("setupText")
            detail.setTextInteractionFlags(
                QtCore.Qt.TextInteractionFlag.TextSelectableByMouse)
            # actions sit under the text, so a narrow window wraps rather
            # than growing a third column
            cell = QtWidgets.QWidget()
            cell.setObjectName("setupActions")
            cell_lay = QtWidgets.QVBoxLayout(cell)
            cell_lay.setContentsMargins(0, 0, 0, 0)
            cell_lay.setSpacing(6)
            cell_lay.addWidget(detail)
            actions = self._actions(item)
            if actions is not None:
                cell_lay.addWidget(actions, 0, QtCore.Qt.AlignmentFlag.AlignLeft)
            self._grid.addWidget(icon, row, 0, QtCore.Qt.AlignmentFlag.AlignTop)
            self._grid.addWidget(title, row, 1)
            self._grid.addWidget(cell, row, 2)

    @staticmethod
    def _badge() -> QtWidgets.QLabel:
        badge = QtWidgets.QLabel()
        badge.setObjectName("setupIcon")
        badge.setFixedSize(20, 20)
        badge.setAlignment(QtCore.Qt.AlignmentFlag.AlignCenter)
        return badge

    @staticmethod
    def _set_status(label: QtWidgets.QLabel, status: str) -> None:
        label.setText(ICONS.get(status, ""))
        label.setProperty("status", status)
        label.style().unpolish(label)
        label.style().polish(label)

    def detail_text(self, item) -> str:
        args = {**self._context, **item.args}
        args["provider"] = PROVIDER_LABELS.get(self._context["provider"],
                                               self._context["provider"])
        if item.key == "vrchat_mic" and "modes" in args:
            args["current"] = ", ".join(
                f"{self._tr('setup_mode_' + mode)}: "
                + (name or self._tr("setup_vrchat_default").format(name=effective or "-"))
                for mode, name, effective in args["modes"])
        try:
            return self._tr(f"setup_{item.key}_{item.state}").format(**args)
        except (KeyError, IndexError, ValueError):
            log.debug("setup text for %s/%s lacks an argument", item.key, item.state)
            return self._tr(f"setup_{item.key}_{item.state}")

    def _actions(self, item):
        buttons = []
        if item.key == "api_key" and item.status != setup_check.OK:
            buttons.append(("setup_btn_get_key", self._open_key_page))
            field = API_KEY_FIELDS.get(self._context["provider"], "api_key")
            buttons.append(("setup_btn_open_settings",
                            lambda: self._on_open_settings(field)))
        elif item.key == "cable" and item.state == "missing":
            if self._on_install_cable is not None and platform_support.IS_WINDOWS:
                buttons.append(("setup_btn_install_cable",
                                lambda: self._on_install_cable()))
            buttons.append(("setup_btn_get_cable", lambda: self._on_open_url(
                QtCore.QUrl(setup_check.VB_CABLE_URL))))
        elif item.key == "system_sound":
            buttons.append(("setup_btn_sound_settings", lambda: self._on_open_url(
                QtCore.QUrl("ms-settings:sound"))))
        elif item.key == "mic":
            buttons.append(("setup_btn_find_mic", lambda: self._on_find_mic()))
        if not buttons:
            return None
        wrap = QtWidgets.QWidget()
        wrap.setObjectName("setupActions")
        lay = QtWidgets.QHBoxLayout(wrap)
        lay.setContentsMargins(0, 0, 0, 0)
        lay.setSpacing(6)
        for key, handler in buttons:
            button = QtWidgets.QPushButton(self._tr(key))
            button.setProperty("actionKey", key)
            button.clicked.connect(lambda _checked=False, h=handler: h())
            lay.addWidget(button)
        return wrap

    def _open_key_page(self) -> None:
        provider = self._context["provider"]
        if provider == "qwen" and self._context["endpoint"] == "beijing":
            url = KEY_URLS["qwen_beijing"]
        else:
            url = KEY_URLS.get(provider, KEY_URLS["gemini"])
        self._on_open_url(QtCore.QUrl(url))
