class Annorex < Formula
  desc "Annotate the replies and files of a coding agent in Rex and send the notes back"
  homepage "https://github.com/devararishivian/annorex"
  url "https://github.com/devararishivian/annorex/archive/refs/tags/v0.2.2.tar.gz"
  sha256 "a3f146932a7c3fad948e0c7c45ae43ed5af0b4748b5c020256d48a73c768a22e"
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
