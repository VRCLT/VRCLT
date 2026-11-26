use super::*;
use crate::update::Status;

impl Studio {
    pub(super) fn update_dialog(&mut self, ctx: &Context) {
        if !self.updates_open {
            return;
        }
        let l = self.lang;
        let mut open = true;
        Window::new(l.t("업데이트", "Updates", "アップデート", "更新"))
            .id(Id::new("vrclt-updates"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.)
            .show(ctx, |ui| {
                ui.label(bold(concat!("VRCLT ", env!("CARGO_PKG_VERSION")), 20.));
                let mut auto = self.cfg.b("/ui/auto_update");
                if ui.checkbox(&mut auto, l.t("자동 업데이트", "Automatic updates", "自動更新", "自动更新")).changed() {
                    let mut cfg = self.cfg.clone();
                    cfg.data["ui"]["auto_update"] = auto.into();
                    if self.save(cfg) {
                        self.draft.data["ui"]["auto_update"] = auto.into();
                        if !auto { self.updates.cancel(); }
                    }
                }
                ui.label(l.t("새 버전을 미리 내려받고 VRCLT를 종료할 때 설치합니다. 트레이에 두는 동안에는 통역을 계속합니다.",
                    "Downloads new versions and installs them when you quit VRCLT. Translation continues in the system tray.",
                    "新しいバージョンをダウンロードし、VRCLT の終了時にインストールします。トレイでは通訳を続けます。",
                    "提前下载新版本，在退出 VRCLT 时安装。最小化到托盘时翻译会继续。"));
                ui.separator();
                match &self.updates.status {
                    Status::Idle => (),
                    Status::Checking => { ui.label(l.t("새 버전 확인 중…", "Checking for updates…", "更新を確認中…", "正在检查更新…")); }
                    Status::Current => { ui.label(l.t("최신 버전입니다.", "You're up to date.", "最新バージョンです。", "已是最新版本。")); }
                    Status::Downloading(percent) => {
                        let label = l.t("업데이트 준비 중", "Preparing update", "更新を準備中", "正在准备更新");
                        ui.add(ProgressBar::new(*percent as f32 / 100.).text(format!("{label}  {percent}%")));
                    }
                    Status::Ready(version, _) => {
                        ui.label(format!("v{version}  {}", l.t("설치 준비 완료", "ready to install", "インストールできます", "可安装")));
                        let label = if self.running {
                            l.t("통역을 종료하고 지금 재시작", "End session and restart now", "通訳を終了して今すぐ再起動", "结束翻译并立即重启")
                        } else {
                            l.t("지금 설치하고 재시작", "Install and restart now", "今すぐ更新して再起動", "立即安装并重启")
                        };
                        if ui.button(label).clicked() && self.updates.install(true) {
                            self.exiting = true;
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                    }
                    Status::Error(error) => {
                        ui.colored_label(ALERT, l.t("업데이트를 완료하지 못했습니다.", "Couldn't complete the update.", "更新を完了できませんでした。", "未能完成更新。"));
                        ui.label(error);
                        ui.hyperlink_to(l.t("공식 다운로드", "Official download", "公式ダウンロード", "官方下载"), "https://github.com/VRCLT/VRCLT/releases/latest");
                    }
                }
                if matches!(self.updates.status, Status::Idle | Status::Current | Status::Error(_))
                    && ui.button(l.t("업데이트 확인 및 다운로드", "Check and download", "更新を確認してダウンロード", "检查并下载更新")).clicked() {
                    self.updates.check(ctx);
                }
            });
        self.updates_open = open;
    }
}
