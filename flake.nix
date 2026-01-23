{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs = inputs@{ self, flake-parts, rust-overlay, ... }:
    flake-parts.lib.mkFlake { inherit self inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem = { self', system, ... }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ rust-overlay.overlays.default ];
          };

          rust-toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          rustPlatform = pkgs.makeRustPlatform {
            cargo = rust-toolchain;
            rustc = rust-toolchain;
          };

        in {
          _module.args.pkgs = pkgs;  # Экспортируем pkgs

          apps.default = {
            type = "app";
            program = self'.packages.default;
          };

          packages = {
            default = self'.packages.bugstalker;
            
            bugstalker = pkgs.callPackage (import ./nix/package.nix) {
              inherit rustPlatform;  # Используем общий rustPlatform
            };
          };

          checks = {
            inherit (self'.packages) bugstalker;
          };

          devShells.default = pkgs.mkShell {
            packages = [ rust-toolchain ];
            
            inputsFrom = [ self'.packages.bugstalker ];
            
            RUST_BACKTRACE = "full";
            RUST_SRC_PATH = "${rust-toolchain}/lib/rustlib/src/rust/library";
          };
        };

      flake = {
        homeManagerModules = rec {
          default = bugstalker;
          bugstalker = import ./nix/home-manager-module.nix self;
        };
      };
    };
}
