"""Soniox voice cloning: the Settings voice field and its dialog.

The dialog lists the project's cloned voices and creates one from a short
microphone recording or an audio file. Network calls run on a worker thread;
their results come back through a queued signal, so nothing here blocks Qt.
"""
import logging
import os
import threading
from pathlib import Path

from PySide6 import QtCore, QtWidgets

from ..audio.voice_sample import VoiceSampleRecorder
from ..soniox import voices as voices_api
from ..soniox.voices import VoiceApiError, VoiceClient
from .widgets import MicLevelMeter

log = logging.getLogger(__name__)

MIN_SAMPLE_SEC = 3.0   # shorter is a mis-click, not a usable reference
AUDIO_PATTERNS = "*.wav *.mp3 *.m4a *.flac *.ogg *.opus *.aac *.webm *.mp4"
RECORDED_FILENAME = "vrclt-voice-sample.wav"


def _clock(seconds: float) -> str:
    seconds = max(0, int(seconds))
    return f"{seconds // 60}:{seconds % 60:02d}"


def _spawn(work) -> None:
    threading.Thread(target=work, daemon=True, name="vrclt-soniox-voices").start()


class VoiceField(QtWidgets.QWidget):
    """Voice name or cloned-voice ID, plus the button that opens voice
    cloning. text()/setText() make it a drop-in for the form's line edits."""

    clone_requested = QtCore.Signal()

    def __init__(self, button_text: str, parent=None):
        super().__init__(parent)
        layout = QtWidgets.QHBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(8)
        self._edit = QtWidgets.QLineEdit()
        self._button = QtWidgets.QPushButton(button_text)
        self._button.clicked.connect(self.clone_requested)
        layout.addWidget(self._edit, 1)
        layout.addWidget(self._button)
        self.setFocusProxy(self._edit)

    def text(self) -> str:
        return self._edit.text()

    def setText(self, text: str) -> None:
        self._edit.setText(text)


class SonioxVoiceDialog(QtWidgets.QDialog):
    _finished = QtCore.Signal(str, object, object)   # operation, result, error

    def __init__(self, tr, *, api_key: str, model: str, current: str = "",
                 mic_device: str = "", threshold: float = 90.0, parent=None,
                 client_factory=VoiceClient, recorder_factory=VoiceSampleRecorder,
                 run_async=_spawn):
        super().__init__(parent)
        self._tr = tr
        self._model = model
        self._current = str(current or "").strip()
        self._mic_device = mic_device
        self._threshold = float(threshold)   # the voice gate, as on the mic meters
        self._client = client_factory(api_key) if str(api_key or "").strip() else None
        self._recorder_factory = recorder_factory
        self._run_async = run_async
        self._recorder = None
        self._sample: bytes | None = None      # a finished recording
        self._sample_path = ""                 # or a chosen file
        self._max_sec = voices_api.MAX_REFERENCE_SEC
        self._supported = True
        self._busy = ""
        self._closed = False
        self._loaded = False
        self.selected = ""                     # voice ID the user chose to use
        self.deleted: set[str] = set()
        self._finished.connect(self._on_finished)
        self._timer = QtCore.QTimer(self)
        self._timer.setInterval(100)
        self._timer.timeout.connect(self._tick)
        self.setWindowTitle(tr("sx_voice_title"))
        self.setMinimumWidth(620)

        layout = QtWidgets.QVBoxLayout(self)
        layout.setSpacing(12)
        self._intro = QtWidgets.QLabel()
        self._intro.setWordWrap(True)
        layout.addWidget(self._intro)

        self._list_box = QtWidgets.QGroupBox()
        list_layout = QtWidgets.QVBoxLayout(self._list_box)
        list_layout.setContentsMargins(12, 4, 12, 6)
        self._list = QtWidgets.QListWidget()
        self._list.setFixedHeight(112)      # three rows; the rest scrolls
        self._list.setAccessibleName(tr("sx_voice_list").format(count=0))
        self._list.itemSelectionChanged.connect(self._sync)
        self._list.itemDoubleClicked.connect(lambda _item: self._use())
        list_layout.addWidget(self._list)
        row = QtWidgets.QHBoxLayout()
        self._btn_refresh = QtWidgets.QPushButton(tr("sx_voice_refresh"))
        self._btn_refresh.clicked.connect(self._load)
        self._btn_delete = QtWidgets.QPushButton(tr("sx_voice_delete"))
        self._btn_delete.clicked.connect(self._delete)
        row.addWidget(self._btn_refresh)
        row.addWidget(self._btn_delete)
        row.addStretch(1)
        list_layout.addLayout(row)
        layout.addWidget(self._list_box)

        new_box = QtWidgets.QGroupBox(tr("sx_voice_new"))
        form = QtWidgets.QFormLayout(new_box)
        form.setContentsMargins(12, 4, 12, 6)
        self._name = QtWidgets.QLineEdit()
        self._name.setMaxLength(voices_api.MAX_NAME_CHARS)
        self._name.textChanged.connect(self._sync)
        form.addRow(tr("sx_voice_name"), self._name)
        row = QtWidgets.QHBoxLayout()
        self._btn_record = QtWidgets.QPushButton(tr("sx_voice_record"))
        self._btn_record.clicked.connect(self._toggle_recording)
        self._btn_file = QtWidgets.QPushButton(tr("sx_voice_file"))
        self._btn_file.clicked.connect(self._choose_file)
        row.addWidget(self._btn_record)
        row.addWidget(self._btn_file)
        row.addStretch(1)
        form.addRow(tr("sx_voice_sample"), row)
        self._meter = MicLevelMeter()
        self._sample_label = QtWidgets.QLabel(tr("sx_voice_no_sample"))
        self._sample_label.setObjectName("noteText")
        self._btn_create = QtWidgets.QPushButton(tr("sx_voice_create"))
        self._btn_create.clicked.connect(self._create)
        row = QtWidgets.QHBoxLayout()
        row.addWidget(self._sample_label, 1)
        row.addWidget(self._meter, 1)
        row.addWidget(self._btn_create)
        form.addRow("", row)
        layout.addWidget(new_box)

        note = QtWidgets.QLabel(tr("sx_voice_note"))
        note.setObjectName("noteText")
        note.setWordWrap(True)
        layout.addWidget(note)
        self._status = QtWidgets.QLabel()
        self._status.setObjectName("setupText")
        self._status.setWordWrap(True)
        self._status.setMinimumHeight(2 * self._status.fontMetrics().lineSpacing())

        self._buttons = QtWidgets.QDialogButtonBox(
            QtWidgets.QDialogButtonBox.StandardButton.Ok
            | QtWidgets.QDialogButtonBox.StandardButton.Close)
        self._btn_use = self._buttons.button(QtWidgets.QDialogButtonBox.StandardButton.Ok)
        self._btn_use.setText(tr("sx_voice_use"))
        self._buttons.button(QtWidgets.QDialogButtonBox.StandardButton.Close).setText(
            tr("sx_voice_close"))
        self._buttons.accepted.connect(self._use)
        self._buttons.rejected.connect(self.reject)
        row = QtWidgets.QHBoxLayout()
        row.addWidget(self._status, 1)
        row.addWidget(self._buttons, 0, QtCore.Qt.AlignmentFlag.AlignBottom)
        layout.addLayout(row)
        self._meter.hide()
        self._set_voices([])
        self._sync()

    # ---------------- state ----------------
    def showEvent(self, event) -> None:
        super().showEvent(event)
        if not self._loaded:
            self._loaded = True
            if self._client is None:
                self._status.setText(self._tr("sx_voice_no_key"))
            else:
                self._load()

    def _voice(self):
        item = self._list.currentItem()
        return item.data(QtCore.Qt.ItemDataRole.UserRole) if item is not None else None

    def _has_sample(self) -> bool:
        return self._sample is not None or bool(self._sample_path)

    def _sync(self) -> None:
        idle = self._client is not None and not self._busy
        recording = self._recorder is not None
        voice = self._voice()
        self._intro.setText(self._tr("sx_voice_intro").format(max=_clock(self._max_sec)))
        self._btn_refresh.setEnabled(idle and not recording)
        self._btn_delete.setEnabled(idle and not recording and voice is not None)
        self._btn_use.setEnabled(
            idle and not recording and voice is not None
            and voice.status(self._model) != voices_api.FAILED)
        can_create = idle and self._supported
        self._name.setEnabled(can_create)
        self._btn_record.setEnabled(can_create)
        self._btn_file.setEnabled(can_create and not recording)
        self._btn_create.setEnabled(
            can_create and not recording and self._has_sample()
            and bool(self._name.text().strip()))

    def _set_voices(self, voices, select: str = "") -> None:
        select = select or (self._voice().id if self._voice() is not None else self._current)
        self._list.blockSignals(True)
        self._list.clear()
        states = {voices_api.READY: "sx_voice_state_ready",
                  voices_api.PROCESSING: "sx_voice_state_processing",
                  voices_api.FAILED: "sx_voice_state_failed"}
        for voice in voices:
            status = voice.status(self._model)
            text = f"{voice.name}  ·  {self._tr(states.get(status, 'sx_voice_state_not_computed'))}"
            if voice.id == self._current:
                text += f"  ·  {self._tr('sx_voice_in_use')}"
            item = QtWidgets.QListWidgetItem(text)
            item.setData(QtCore.Qt.ItemDataRole.UserRole, voice)
            item.setToolTip("\n".join(part for part in (
                voice.id, voice.filename, voice.created_at, voice.error(self._model)) if part))
            self._list.addItem(item)
            if voice.id == select:
                self._list.setCurrentItem(item)
        self._list.blockSignals(False)
        self._list_box.setTitle(self._tr("sx_voice_list").format(count=len(voices)))
        self._sync()

    # ---------------- worker plumbing ----------------
    def _run(self, operation: str, job, status_key: str) -> None:
        self._busy = operation
        self._status.setText(self._tr(status_key))
        self._sync()

        def work():
            try:
                result, error = job(), None
            except Exception as e:   # reported in the dialog, never raised into Qt
                if not isinstance(e, (VoiceApiError, OSError)):
                    log.exception("Soniox voice %s failed", operation)
                result, error = None, e
            if self._closed:
                return
            try:
                self._finished.emit(operation, result, error)
            except RuntimeError:
                pass    # the dialog was destroyed while the request ran

        self._run_async(work)

    def _on_finished(self, operation: str, result, error) -> None:
        if self._closed:
            return
        self._busy = ""
        if error is not None:
            self._status.setText(self._tr("sx_voice_error").format(detail=str(error)))
            self._sync()
            return
        if operation == "load":
            voices, limit = result
            if limit is not None:
                self._supported = limit > 0
                self._max_sec = limit if limit > 0 else self._max_sec
            self._status.setText("" if self._supported else self._tr("sx_voice_unsupported"))
            self._set_voices(voices)
        elif operation == "create":
            voice, failure, voices = result
            self._set_voices(voices, select=voice.id)
            if failure is not None:
                self._status.setText(self._tr("sx_voice_error").format(detail=str(failure)))
            else:
                self._sample, self._sample_path = None, ""
                self._sample_label.setText(self._tr("sx_voice_no_sample"))
                self._name.clear()
                self._status.setText(self._tr("sx_voice_created").format(name=voice.name))
        elif operation == "delete":
            voice_id, voices = result
            self.deleted.add(voice_id)
            self._status.setText(self._tr("sx_voice_deleted"))
            self._set_voices(voices)
        elif operation == "use":
            self.selected = result.id
            super().accept()
            return
        self._sync()

    # ---------------- actions ----------------
    def _load(self) -> None:
        client, model = self._client, self._model

        def job():
            try:
                limit = client.max_reference_sec(model)
            except VoiceApiError:
                limit = None    # the list still matters; keep the default limit
            return client.list(), limit

        self._run("load", job, "sx_voice_loading")

    def _toggle_recording(self) -> None:
        if self._recorder is None:
            recorder = self._recorder_factory(self._mic_device, self._max_sec)
            try:
                recorder.start()
            except Exception as e:
                log.info("voice sample: microphone unavailable (%s)", e)
                self._status.setText(self._tr("sx_voice_mic_failed").format(detail=str(e)))
                return
            self._recorder = recorder
            self._sample, self._sample_path = None, ""
            self._btn_record.setText(self._tr("sx_voice_stop"))
            self._status.setText("")
            self._meter.show()
            self._timer.start()
            self._tick()
        else:
            self._stop_recording()
        self._sync()

    def _stop_recording(self) -> None:
        recorder, self._recorder = self._recorder, None
        self._timer.stop()
        self._meter.hide()
        self._btn_record.setText(self._tr("sx_voice_record"))
        if recorder is None:
            return
        recorder.stop()
        if recorder.seconds < MIN_SAMPLE_SEC:
            self._sample_label.setText(self._tr("sx_voice_no_sample"))
            self._status.setText(self._tr("sx_voice_too_short").format(
                seconds=f"{MIN_SAMPLE_SEC:g}"))
        else:
            self._sample = recorder.wav_bytes()
            self._sample_label.setText(
                self._tr("sx_voice_recorded").format(time=_clock(recorder.seconds)))
        self._sync()

    def _tick(self) -> None:
        recorder = self._recorder
        if recorder is None:
            return
        self._meter.set_level(recorder.level, self._threshold)
        self._sample_label.setText(self._tr("sx_voice_recording").format(
            time=_clock(recorder.seconds), max=_clock(self._max_sec)))
        if recorder.full:
            self._stop_recording()

    def _choose_file(self) -> None:
        path, _filter = QtWidgets.QFileDialog.getOpenFileName(
            self, self._tr("sx_voice_file"), "",
            f"{self._tr('sx_voice_file_filter')} ({AUDIO_PATTERNS});;* (*)")
        if not path:
            return
        try:
            size = os.path.getsize(path)
        except OSError as e:
            self._status.setText(self._tr("sx_voice_error").format(detail=str(e)))
            return
        if size > voices_api.MAX_UPLOAD_BYTES:
            self._status.setText(self._tr("sx_voice_too_large"))
            return
        self._sample, self._sample_path = None, path
        self._sample_label.setText(Path(path).name)
        self._status.setText("")
        if not self._name.text().strip():
            self._name.setText(Path(path).stem[:voices_api.MAX_NAME_CHARS])
        self._sync()

    def _create(self) -> None:
        client, model = self._client, self._model
        name = self._name.text().strip()
        sample, path = self._sample, self._sample_path

        def job():
            audio = sample if sample is not None else Path(path).read_bytes()
            voice = client.create(name, audio, RECORDED_FILENAME if sample is not None
                                  else Path(path).name)
            failure = None
            try:
                voice = client.wait_ready(voice.id, model, cancelled=lambda: self._closed)
            except VoiceApiError as e:
                failure = e     # the voice exists; show it with its state
            return voice, failure, client.list()

        self._run("create", job, "sx_voice_creating")

    def _delete(self) -> None:
        voice = self._voice()
        if voice is None:
            return
        reply = QtWidgets.QMessageBox.question(
            self, self._tr("sx_voice_title"),
            self._tr("sx_voice_delete_confirm").format(name=voice.name))
        if reply != QtWidgets.QMessageBox.StandardButton.Yes:
            return
        client = self._client

        def job():
            client.delete(voice.id)
            return voice.id, client.list()

        self._run("delete", job, "sx_voice_deleting")

    def _use(self) -> None:
        voice = self._voice()
        if voice is None or self._busy or not self._btn_use.isEnabled():
            return
        if voice.status(self._model) == voices_api.READY:
            self.selected = voice.id
            super().accept()
            return
        client, model = self._client, self._model
        self._run("use", lambda: client.wait_ready(
            voice.id, model, cancelled=lambda: self._closed), "sx_voice_preparing")

    def done(self, result: int) -> None:
        self._closed = True
        self._timer.stop()
        if self._recorder is not None:
            self._recorder.stop()
            self._recorder = None
        super().done(result)
