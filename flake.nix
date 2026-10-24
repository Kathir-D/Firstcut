# Firstcut — a native photo culler (the v2 pure-Rust rewrite)
#
# NOTE: this flake is NOT a supported build lane yet (docs/v2-plan.md §8). It
# builds the desktop app from the workspace; the repo pins Rust 1.99.0 via
# rust-toolchain.toml, whereas this flake uses nixpkgs' rustc — treat it as a
# convenience, and packages from the release workflow as the real artefacts.
{
  description = "Firstcut — cull photography shoots natively, fast, on your machine";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: rec {
        firstcut = pkgs.rustPlatform.buildRustPackage {
          pname = "firstcut";
          version = "0.2.0-dev";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          # eframe/egui need the pointer/keyboard libraries on Linux
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = with pkgs;
            pkgs.lib.optionals pkgs.stdenv.isLinux [
              libx11
              libxcursor
              libxkbcommon
              wayland
            ];
        };
        default = firstcut;
      });
    };
}
