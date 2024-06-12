self: { config, lib, pkgs, ... }:
with lib;
let
  bugstalker = self.packages.${pkgs.stdenv.hostPlatform.system}.bugstalker;
  pkgs_with_bugstalker = pkgs // { inherit bugstalker; };

  cfg = config.programs.bugstalker;
  tomlFormat = pkgs.formats.toml { };
in
{
  options.programs.bugstalker = {
    enable = mkEnableOption "Bugstalker";

    package = mkPackageOption pkgs_with_bugstalker "Bugstalker" {
      default = [ bugstalker.pname ];
    };

    keymap = mkOption {
      type = tomlFormat.type;
      default = { };
    };
  };

  config = mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile."bs/keymap.toml" = lib.mkIf (cfg.keymap != { }) {
      source = (tomlFormat.generate "keymap.toml" cfg.keymap);
    };
  };
}
