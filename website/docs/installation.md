---
sidebar_position: 2
---

# Installation

## Install from sources

Just use `cargo install` command:

```shell
cargo install bugstalker
```

That's all, the `bs` command is available now!

<details>
  <summary>Use `libunwind`</summary>

By default, BS uses a built-in unwinder, but you can use `libunwind` instead.
Note that this may be risky because `libunwind` support in BS is deprecated.

First, check if the necessary dependencies
(`pkg-config` and `libunwind-dev`) are installed:

For example, on Ubuntu/Debian:

```shell
apt install pkg-config libunwind-dev
```
Now install the debugger:

```shell
cargo install bugstalker --features libunwind
```
</details>

## Distro Packages

<details>
  <summary>Packaging status</summary>

[![Packaging status](https://repology.org/badge/vertical-allrepos/bugstalker.svg)](https://repology.org/project/bugstalker/versions)

</details>

### Arch Linux

```shell
pacman -S bugstalker
```

## Nix package manager

There's flake which you can use to start using it.
Just [enable flakes](https://wiki.nixos.org/wiki/Flakes#Enable_flakes_temporarily)
then you're able to use it with:

```
nix run github:godzie44/BugStalker
```

`BugStalker` also provides a package which you can include in your NixOS config.
For example:

<details>

```nix
{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    bugstalker.url = "github:godzie44/BugStalker";
  };

  outpus = {nixpkgs, bugstalker, ...}: {
    nixosConfigurations.your_hostname = nixpkgs.lib.nixosSystem {
      modules = [
        ({...}: {
          environment.systemPackages = [
            # assuming your system runs on a x86-64 cpu
            bugstalker.packages."x86_64-linux".default
          ];
        })
      ];
    };
  };
}
```

</details>

### Home-Manager

There's a home-manager module which adds `programs.bugstalker` to your home-manager config.
You can add it by doing the following:

<details>

```nix
{
  description = "NixOS configuration";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
    home-manager.inputs.nixpkgs.follows = "nixpkgs";
    bugstalker.url = "github:godzie44/BugStalker";
  };

  outputs = inputs@{ nixpkgs, home-manager, bugstalker, ... }: {
    nixosConfigurations = {
      hostname = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          ./configuration.nix
          home-manager.nixosModules.home-manager
          {
            home-manager.sharedModules = [
              bugstalker.homeManagerModules.default
              ({...}: {
                programs.bugstalker = {
                  enable = true;
                  # the content of `keymap.toml`
                  keymap = {
                    common = {
                      up = ["k"];
                    }
                  };
                };
              })
            ];
          }
        ];
      };
    };
  };
}
```

</details>
