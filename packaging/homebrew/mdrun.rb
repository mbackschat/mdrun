class Mdrun < Formula
  desc "Create, run, and verify executable Markdown documents"
  homepage "https://github.com/mbackschat/mdrun"
  version "{{version}}"
  license "MIT"

  head do
    url "https://github.com/mbackschat/mdrun.git", branch: "main"
    depends_on "rust" => :build
  end

  on_macos do
    on_arm do
      url "https://github.com/mbackschat/mdrun/releases/download/v{{version}}/mdrun-v{{version}}-aarch64-apple-darwin.tar.gz"
      sha256 "{{sha256_aarch64_apple_darwin}}"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/mbackschat/mdrun/releases/download/v{{version}}/mdrun-v{{version}}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "{{sha256_x86_64_unknown_linux_musl}}"
    end
    on_arm do
      url "https://github.com/mbackschat/mdrun/releases/download/v{{version}}/mdrun-v{{version}}-aarch64-unknown-linux-musl.tar.gz"
      sha256 "{{sha256_aarch64_unknown_linux_musl}}"
    end
  end

  def install
    if build.head?
      system "cargo", "install", *std_cargo_args
    else
      bin.install "mdrun"
    end
    prefix.install "LICENSE"
  end

  test do
    output = with_env(PATH: "") { shell_output("#{bin}/mdrun --version") }
    assert_match(/^mdrun \d+\.\d+\.\d+\n$/, output)
    assert_match "executable Markdown", shell_output("#{bin}/mdrun --help")
  end
end
