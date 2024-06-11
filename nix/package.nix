{ lib
, rustPlatform
, pkg-config
, libunwind
}:
let
  cargoToml = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = cargoToml.package.name;
  version = cargoToml.package.version;

  src = builtins.path {
    path = ../.;
  };

  cargoLock.lockFile = ../Cargo.lock;

  buildInputs = [ libunwind ];

  nativeBuildInputs = [ pkg-config ];

  # See https://github.com/NixOS/nixpkgs/blob/nixos-24.05/pkgs/by-name/bu/bugstalker/package.nix#L25-L26
  doCheck = false;

  meta = {
    description = "Rust debugger for Linux x86-64";
    homepage = "https://github.com/godzie44/BugStalker";
    license = lib.licenses.mit;
    mainProgram = "bs";
    platforms = [ "x86_64-linux" ];
  };
}
