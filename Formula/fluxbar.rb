# Homebrew formula for FluxBar.
#
# This repository is also a valid Homebrew tap, so a development build can be
# installed directly from a checkout:
#
#   brew tap fluxbar/dev https://github.com/fluxbar/fluxbar.git
#   brew install --HEAD fluxbar/dev/fluxbar
#   brew services start fluxbar
#
# A tagged release adds a stable `url`/`sha256` pair and is published through
# the `fluxbar/tap` tap. Until then, use `--HEAD`.
class Fluxbar < Formula
  desc "Native macOS status bar with a Liquid Glass UI and plugins"
  homepage "https://github.com/fluxbar/fluxbar"
  version "0.1.0"
  license "MIT"
  head "https://github.com/fluxbar/fluxbar.git", branch: "master"

  depends_on "rust" => :build
  depends_on macos: :monterey

  def fetch
    system "cargo", "fetch", *std_cargo_fetch_args
  end

  def install
    system "cargo", "install", *std_cargo_args(path: "fluxbar-cli")
  end

  service do
    run [opt_bin/"fluxbar", "start"]
    # Restart after a crash, but let a clean `fluxbar stop` stay stopped until
    # the next login or `brew services restart`.
    keep_alive crashed: true
    run_at_load true
    process_type :interactive
    stop_timeout 10
    working_dir var
    log_path var/"log/fluxbar.log"
    error_log_path var/"log/fluxbar.err.log"
    environment_variables PATH: std_service_path_env
  end

  def caveats
    <<~EOS
      FluxBar reads ~/.config/fluxbar/fluxbar.toml and creates a default
      configuration on first start.

      Start FluxBar at login with:
        brew services start fluxbar

      Stop the managed service with:
        brew services stop fluxbar

      Service logs:
        #{var}/log/fluxbar.log
        #{var}/log/fluxbar.err.log
    EOS
  end

  test do
    assert_match(/\Afluxbar \d+\.\d+\.\d+/, shell_output("#{bin}/fluxbar --version"))
  end
end
