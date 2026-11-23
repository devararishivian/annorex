class Annorex < Formula
  desc "Annotate the replies and files of a coding agent in Rex and send the notes back"
  homepage "https://github.com/devararishivian/annorex"
  url "https://github.com/devararishivian/annorex/archive/refs/tags/v0.4.0.tar.gz"
  sha256 "c838c4bb6301d1811660e5b0743d30381826b1b29fdd0eb9f53a2ce2e0ee00ca"
  license "MIT"

  depends_on "pkgconf" => :build
  depends_on "rust" => :build
  depends_on :macos

  uses_from_macos "sqlite"

  deny_network_access!

  def fetch
    system "cargo", "fetch", *std_cargo_fetch_args
  end

  def install
    ENV["ANNOREX_DISABLE_UPDATE_CHECK"] = "1"
    ENV["LIBSQLITE3_SYS_USE_PKG_CONFIG"] = "1"
    system "cargo", "install", *std_cargo_args(path: "crates/annorex")
  end

  test do
    system bin/"annorex", "rex", "install", "--dir", testpath
    assert_match "Installed by `annorex rex install`", (testpath/"annorex.lua").read
    assert_match "--check", shell_output("#{bin}/annorex update --help")
  end
end
