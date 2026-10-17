"""Dialog that finds the microphone the user is speaking into."""
from PySide6 import QtCore, QtWidgets

from ..audio.mic_probe import MicProbe
from .widgets import MicLevelMeter


class MicFinderDialog(QtWidgets.QDialog):
    """Live meters for every real microphone; the one that rises most while
    the user speaks is preselected. Clicking another row overrides it."""

    def __init__(self, tr, names, *, current: str = "", threshold: float = 90.0,
                 parent=None, probe_factory=MicProbe):
        super().__init__(parent)
        self._tr = tr
        self._threshold = float(threshold)
        self._probe = probe_factory(names)
        self._manual = False
        self._started = False
        self.selected = ""
        self._timer = QtCore.QTimer(self)
        self._timer.setInterval(100)
        self._timer.timeout.connect(self._tick)
        self.setWindowTitle(tr("mic_finder_title"))
        self.setMinimumWidth(520)

        self._buttons = QtWidgets.QDialogButtonBox(
            QtWidgets.QDialogButtonBox.StandardButton.Ok
            | QtWidgets.QDialogButtonBox.StandardButton.Cancel)
        self._buttons.button(QtWidgets.QDialogButtonBox.StandardButton.Ok).setText(
            tr("mic_finder_use"))
        self._buttons.accepted.connect(self.accept)
        self._buttons.rejected.connect(self.reject)

        layout = QtWidgets.QVBoxLayout(self)
        layout.setSpacing(12)
        intro = QtWidgets.QLabel(tr("mic_finder_intro"))
        intro.setWordWrap(True)
        layout.addWidget(intro)
        grid = QtWidgets.QGridLayout()
        grid.setHorizontalSpacing(12)
        grid.setVerticalSpacing(8)
        self._group = QtWidgets.QButtonGroup(self)
        self._rows = {}
        for row, name in enumerate(self._probe.names):
            radio = QtWidgets.QRadioButton(name)
            radio.setProperty("device", name)
            radio.toggled.connect(self._row_toggled)
            meter = MicLevelMeter()
            meter.setMinimumWidth(160)
            self._group.addButton(radio)
            grid.addWidget(radio, row, 0)
            grid.addWidget(meter, row, 1)
            self._rows[name] = (radio, meter)
            if name == current:
                radio.setChecked(True)
        grid.setColumnStretch(1, 1)
        layout.addLayout(grid)
        self._status = QtWidgets.QLabel(
            tr("mic_finder_listening") if self._rows else tr("mic_finder_no_devices"))
        self._status.setObjectName("setupText")
        self._status.setWordWrap(True)
        layout.addWidget(self._status)
        layout.addWidget(self._buttons)
        self._sync_ok()

    def showEvent(self, event) -> None:
        super().showEvent(event)
        if not self._started:
            self._started = True
            self._probe.start()
            for name in self._probe.failed:
                radio, meter = self._rows.get(name, (None, None))
                if radio is not None:
                    radio.setEnabled(False)
                    radio.setText(f"{name} ({self._tr('mic_finder_unavailable')})")
            self._timer.start()

    def _row_toggled(self, checked: bool) -> None:
        if checked and self._timer.isActive():
            self._manual = True
        self._sync_ok()

    def _sync_ok(self) -> None:
        self._buttons.button(QtWidgets.QDialogButtonBox.StandardButton.Ok).setEnabled(
            self._group.checkedButton() is not None)

    def _tick(self) -> None:
        for name, (_radio, meter) in self._rows.items():
            level = self._probe.levels.get(name)
            meter.set_level(level, self._threshold if level is not None else None)
        best = self._probe.best()
        if best:
            self._status.setText(self._tr("mic_finder_found").format(name=best))
            if not self._manual:
                radio = self._rows[best][0]
                blocked = radio.blockSignals(True)
                radio.setChecked(True)
                radio.blockSignals(blocked)
                self._sync_ok()

    def accept(self) -> None:
        checked = self._group.checkedButton()
        self.selected = str(checked.property("device")) if checked is not None else ""
        super().accept()

    def done(self, result: int) -> None:
        self._timer.stop()
        self._probe.stop()
        super().done(result)
