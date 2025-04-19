"use strict";(self.webpackChunkwebsite=self.webpackChunkwebsite||[]).push([[803],{23:(e,n,s)=>{s.r(n),s.d(n,{assets:()=>o,contentTitle:()=>l,default:()=>d,frontMatter:()=>t,metadata:()=>a,toc:()=>c});const a=JSON.parse('{"id":"installation","title":"Installation","description":"Install from sources","source":"@site/docs/installation.md","sourceDirName":".","slug":"/installation","permalink":"/BugStalker/docs/installation","draft":false,"unlisted":false,"editUrl":"https://github.com/facebook/docusaurus/tree/main/packages/create-docusaurus/templates/shared/docs/installation.md","tags":[],"version":"current","sidebarPosition":2,"frontMatter":{"sidebar_position":2},"sidebar":"tutorialSidebar","previous":{"title":"Overview","permalink":"/BugStalker/docs/overview"},"next":{"title":"Supported rustc versions","permalink":"/BugStalker/docs/support-rustc"}}');var i=s(4848),r=s(8453);const t={sidebar_position:2},l="Installation",o={},c=[{value:"Install from sources",id:"install-from-sources",level:2},{value:"Distro Packages",id:"distro-packages",level:2},{value:"Arch Linux",id:"arch-linux",level:3},{value:"Nix package manager",id:"nix-package-manager",level:2},{value:"Home-Manager",id:"home-manager",level:3}];function u(e){const n={a:"a",code:"code",h1:"h1",h2:"h2",h3:"h3",header:"header",img:"img",p:"p",pre:"pre",...(0,r.R)(),...e.components},{Details:s}=n;return s||function(e,n){throw new Error("Expected "+(n?"component":"object")+" `"+e+"` to be defined: you likely forgot to import, pass, or provide it.")}("Details",!0),(0,i.jsxs)(i.Fragment,{children:[(0,i.jsx)(n.header,{children:(0,i.jsx)(n.h1,{id:"installation",children:"Installation"})}),"\n",(0,i.jsx)(n.h2,{id:"install-from-sources",children:"Install from sources"}),"\n",(0,i.jsxs)(n.p,{children:["First, check if the necessary dependencies\n(",(0,i.jsx)(n.code,{children:"pkg-config"})," and ",(0,i.jsx)(n.code,{children:"libunwind-dev"}),") are installed:"]}),"\n",(0,i.jsx)(n.p,{children:"For example, on Ubuntu/Debian:"}),"\n",(0,i.jsx)(n.pre,{children:(0,i.jsx)(n.code,{className:"language-shell",children:"apt install pkg-config libunwind-dev\n"})}),"\n",(0,i.jsx)(n.p,{children:"Now install the debugger:"}),"\n",(0,i.jsx)(n.pre,{children:(0,i.jsx)(n.code,{className:"language-shell",children:"cargo install bugstalker\n"})}),"\n",(0,i.jsxs)(n.p,{children:["That's all, the ",(0,i.jsx)(n.code,{children:"bs"})," command is available now!"]}),"\n",(0,i.jsxs)(s,{children:[(0,i.jsx)("summary",{children:"Problem with libunwind?"}),(0,i.jsxs)(n.p,{children:["If you have any issues with ",(0,i.jsx)(n.code,{children:"libunwind"}),", you can try to install ",(0,i.jsx)(n.code,{children:"bs"})," with\na native unwinder\n(currently, I don't recommend this method because libunwind is better :))"]}),(0,i.jsx)(n.pre,{children:(0,i.jsx)(n.code,{className:"language-shell",children:"cargo install bugstalker --no-default-features\n"})})]}),"\n",(0,i.jsx)(n.h2,{id:"distro-packages",children:"Distro Packages"}),"\n",(0,i.jsxs)(s,{children:[(0,i.jsx)("summary",{children:"Packaging status"}),(0,i.jsx)(n.p,{children:(0,i.jsx)(n.a,{href:"https://repology.org/project/bugstalker/versions",children:(0,i.jsx)(n.img,{src:"https://repology.org/badge/vertical-allrepos/bugstalker.svg",alt:"Packaging status"})})})]}),"\n",(0,i.jsx)(n.h3,{id:"arch-linux",children:"Arch Linux"}),"\n",(0,i.jsx)(n.pre,{children:(0,i.jsx)(n.code,{className:"language-shell",children:"pacman -S bugstalker\n"})}),"\n",(0,i.jsx)(n.h2,{id:"nix-package-manager",children:"Nix package manager"}),"\n",(0,i.jsxs)(n.p,{children:["There's flake which you can use to start using it.\nJust ",(0,i.jsx)(n.a,{href:"https://wiki.nixos.org/wiki/Flakes#Enable_flakes_temporarily",children:"enable flakes"}),"\nthen you're able to use it with:"]}),"\n",(0,i.jsx)(n.pre,{children:(0,i.jsx)(n.code,{children:"nix run github:godzie44/BugStalker\n"})}),"\n",(0,i.jsxs)(n.p,{children:[(0,i.jsx)(n.code,{children:"BugStalker"})," also provides a package which you can include in your NixOS config.\nFor example:"]}),"\n",(0,i.jsx)(s,{children:(0,i.jsx)(n.pre,{children:(0,i.jsx)(n.code,{className:"language-nix",children:'{\n  inputs = {\n    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";\n    bugstalker.url = "github:godzie44/BugStalker";\n  };\n\n  outpus = {nixpkgs, bugstalker, ...}: {\n    nixosConfigurations.your_hostname = nixpkgs.lib.nixosSystem {\n      modules = [\n        ({...}: {\n          environment.systemPackages = [\n            # assuming your system runs on a x86-64 cpu\n            bugstalker.packages."x86_64-linux".default\n          ];\n        })\n      ];\n    };\n  };\n}\n'})})}),"\n",(0,i.jsx)(n.h3,{id:"home-manager",children:"Home-Manager"}),"\n",(0,i.jsxs)(n.p,{children:["There's a home-manager module which adds ",(0,i.jsx)(n.code,{children:"programs.bugstalker"})," to your home-manager config.\nYou can add it by doing the following:"]}),"\n",(0,i.jsx)(s,{children:(0,i.jsx)(n.pre,{children:(0,i.jsx)(n.code,{className:"language-nix",children:'{\n  description = "NixOS configuration";\n\n  inputs = {\n    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";\n    home-manager.url = "github:nix-community/home-manager";\n    home-manager.inputs.nixpkgs.follows = "nixpkgs";\n    bugstalker.url = "github:godzie44/BugStalker";\n  };\n\n  outputs = inputs@{ nixpkgs, home-manager, bugstalker, ... }: {\n    nixosConfigurations = {\n      hostname = nixpkgs.lib.nixosSystem {\n        system = "x86_64-linux";\n        modules = [\n          ./configuration.nix\n          home-manager.nixosModules.home-manager\n          {\n            home-manager.sharedModules = [\n              bugstalker.homeManagerModules.default\n              ({...}: {\n                programs.bugstalker = {\n                  enable = true;\n                  # the content of `keymap.toml`\n                  keymap = {\n                    common = {\n                      up = ["k"];\n                    }\n                  };\n                };\n              })\n            ];\n          }\n        ];\n      };\n    };\n  };\n}\n'})})})]})}function d(e={}){const{wrapper:n}={...(0,r.R)(),...e.components};return n?(0,i.jsx)(n,{...e,children:(0,i.jsx)(u,{...e})}):u(e)}},8453:(e,n,s)=>{s.d(n,{R:()=>t,x:()=>l});var a=s(6540);const i={},r=a.createContext(i);function t(e){const n=a.useContext(r);return a.useMemo((function(){return"function"==typeof e?e(n):{...n,...e}}),[n,e])}function l(e){let n;return n=e.disableParentContext?"function"==typeof e.components?e.components(i):e.components||i:t(e.components),a.createElement(r.Provider,{value:n},e.children)}}}]);