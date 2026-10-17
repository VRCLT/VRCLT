"""An app upgrade must replace SteamVR's persisted thumbnail, without a headset."""
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from PIL import Image
from vrclt.resources import icon_path
from vrclt.vr import dashboard_panel


def main():
    bundled = icon_path("icon.png")
    with tempfile.TemporaryDirectory(prefix="vrclt-icon-check-") as directory:
        cached = Path(directory) / "profile" / "dashboard_icon.png"
        with patch.object(dashboard_panel, "ICON_PATH", cached):
            # A fresh install creates the thumbnail and its parent directory.
            assert dashboard_panel._ensure_icon()
            assert cached.read_bytes() == bundled.read_bytes()

            # An older installed version left a different icon in the profile.
            Image.new("RGBA", (256, 256), "red").save(cached)
            assert dashboard_panel._ensure_icon()
            assert cached.read_bytes() == bundled.read_bytes()

            # Matching content needs no write (even after source timestamps change).
            with patch.object(dashboard_panel.shutil, "copyfile") as copyfile:
                assert dashboard_panel._ensure_icon()
                copyfile.assert_not_called()

            # Retain a usable cache when the bundled resource is unavailable.
            with patch.object(dashboard_panel, "icon_asset_path",
                              return_value=Path(directory) / "missing.png"):
                assert dashboard_panel._ensure_icon()
                assert cached.read_bytes() == bundled.read_bytes()
                cached.unlink()
                assert dashboard_panel._ensure_icon()
                with Image.open(cached) as fallback:
                    assert fallback.size == (256, 256)

    print("smoke_dashboard_icon: OK (fresh install, upgrade, unchanged, fallback)")


if __name__ == "__main__":
    main()
