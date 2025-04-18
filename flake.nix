{
  description = "cgt-tools";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
    pre-commit-hooks-nix = {
      url = "github:cachix/pre-commit-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.nixpkgs-stable.follows = "nixpkgs";
    };
  };
  outputs =
    inputs@{ self, ... }:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } (
      { withSystem, ... }:
      {
        imports = [
          inputs.pre-commit-hooks-nix.flakeModule
        ];

        systems = inputs.nixpkgs.lib.systems.flakeExposed;

        perSystem =
          {
            config,
            self',
            inputs',
            pkgs,
            lib,
            system,
            ...
          }:
          {
            pre-commit.settings = {
              src = ./.;
              hooks = {
                nixfmt-rfc-style.enable = true;
                rustfmt.enable = true;
              };
            };

            packages = {
              default = self'.packages.nix-transform;
              nix-transform = pkgs.rustPlatform.buildRustPackage {
                pname = "nix-transform";
                version =
                  (builtins.listToAttrs (
                    builtins.map (p: {
                      inherit (p) name;
                      value = p;
                    }) ((builtins.fromTOML (builtins.readFile ./Cargo.lock)).package)
                  )).nix-transform.version;

                cargoLock.lockFile = ./Cargo.lock;

                src = ./.;

                nativeBuildInputs = [
                  pkgs.clang
                ];

                env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
              };
            };

            devShells.default = pkgs.mkShell {
              shellHook = ''
                ${config.pre-commit.installationScript}
              '';

              hardeningDisable = [ "fortify" ];
              env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";

              nativeBuildInputs = [
                pkgs.rust-analyzer
                pkgs.cargo
                pkgs.rustc
                pkgs.rustfmt
                pkgs.clang
              ];
            };
          };
      }
    );
}
