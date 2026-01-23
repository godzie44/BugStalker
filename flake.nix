{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs =
    inputs@{
      self,
      flake-parts,
      rust-overlay,
      ...
    }:
    flake-parts.lib.mkFlake { inherit self inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];

      perSystem =
        {
          self',
          system,
          pkgs,
          ...
        }:
        {
          _module.args.pkgs = import inputs.nixpkgs {
            inherit system;

            overlays = [
              rust-overlay.overlays.default
            ];
          };

          apps.default = {
            type = "app";
            program = self'.packages.default;
          };

          packages = {
            default = self'.packages.bugstalker;
            bugstalker = pkgs.callPackage (import ./nix/package.nix) {
              rustPlatform =
                let
                  rust-bin = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
                in
                pkgs.makeRustPlatform {
                  cargo = rust-bin;
                  rustc = rust-bin;
                };
            };
          };

          checks = {
            inherit (self'.packages) bugstalker;
          };

          devShells.default =
            let
              bs = self'.packages.default;
              rust-toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
            in
            pkgs.mkShell {
              packages = [ rust-toolchain ] ++ bs.buildInputs ++ bs.nativeBuildInputs;
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
