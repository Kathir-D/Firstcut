cask "firstcut" do
  # Both values are filled in by .github/workflows/release.yml on every v* tag: the version comes
  # from the tag, the SHA-256 from the zip that run produced. The placeholders below are what makes
  # `brew install --cask firstcut` fail loudly instead of installing something unchecked.
  version "0.0.0"
  sha256 "REPLACE_WITH_RELEASE_SHA256"

  url "https://github.com/Kathir-D/Firstcut/releases/download/v#{version}/Firstcut-#{version}.zip"
  name "Firstcut"
  desc "Hyper-fast manual photo culler: bursts, keyboard, no waiting"
  homepage "https://github.com/Kathir-D/Firstcut"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: :sequoia

  app "Firstcut.app"

  # Firstcut is ad-hoc signed and not notarized (no paid Apple Developer account), so Gatekeeper
  # would otherwise make every user approve it by hand in System Settings on first launch.
  #
  # Homebrew used to offer --no-quarantine for this, but it was removed in 7.x
  # ("Error: invalid option: --no-quarantine") and there is no cask DSL replacement:
  # `postflight_steps` exposes only if_path_exists / on_macos / version / token, and cannot run a
  # command at all. The deprecated `postflight` block can, and still runs.
  #
  # The rescue matters. Without it, removing `postflight` would not merely stop the quarantine from
  # being cleared - it would make the cask file invalid, and an invalid cask stops the whole tap
  # from loading, so `brew tap` would fail and nobody could install anything. Rescued, the worst
  # case is that users get the normal one-time Gatekeeper approval again.
  #
  # The ordering is what makes this reasonable rather than reckless: Homebrew verifies the SHA-256
  # above before any of this runs, so the checksum is the integrity gate and the quarantine
  # attribute is no longer what stands between the user and an app they knowingly installed.
  #
  # `brew style` reports one offense on the block below, Cask/InstallSteps, and it cannot be
  # resolved: Homebrew requires postflight_steps, whose DSL cannot run a command at all, and
  # Style/DisableCopsWithinSourceCodeDirective forbids suppressing the cop. The two rules together
  # make the requirement unsatisfiable, so the block stays and the offense stays. Check with:
  #   brew style --except Cask/InstallSteps kathir-d/tap/firstcut
  # `brew audit --cask --strict`, the gate Homebrew actually enforces, passes.
  #
  begin
    postflight do
      system_command(
        "/usr/bin/xattr",
        args:         ["-dr", "com.apple.quarantine", "/Applications/Firstcut.app"],
        must_succeed: false,
      )
    end
  rescue NoMethodError
    # Homebrew dropped the postflight block. Nothing to do; the install itself still succeeds.
  end

  uninstall quit: "com.kathird.firstcut"

  zap trash: [
    "~/Library/Application Support/Firstcut",
    "~/Library/Caches/com.kathird.firstcut",
    "~/Library/Logs/Firstcut",
    "~/Library/Preferences/com.kathird.firstcut.plist",
    "~/Library/Saved Application State/com.kathird.firstcut.savedState",
  ]
end
