{
  description = "Native GPUI frontend for T3 Code";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix.url = "github:numtide/treefmt-nix";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-darwin"
      ];

      imports = [ inputs.treefmt-nix.flakeModule ];

      perSystem =
        { pkgs, lib, ... }:
        let
          runtimeLibs = lib.optionals pkgs.stdenv.hostPlatform.isLinux (
            with pkgs;
            [
              vulkan-loader
              wayland
              libxkbcommon
              libxcb
              libx11
              fontconfig
              freetype
            ]
          );
          buildLibs =
            runtimeLibs
            ++ (with pkgs; [
              aws-lc
              openssl
              zlib
            ])
            ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.alsa-lib ];
          nativeTools = with pkgs; [
            pkg-config
            cmake
            perl
            rustPlatform.bindgenHook
          ];
        in
        {
          packages.default = pkgs.rustPlatform.buildRustPackage {
            pname = "t3code-gpui";
            version = "0.1.0";
            src = lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "--package"
              "t3code-gpui"
            ];
            doCheck = false;
            nativeBuildInputs = nativeTools ++ [ pkgs.makeWrapper ];
            buildInputs = buildLibs;
            postFixup = lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              wrapProgram $out/bin/t3code-gpui \
                --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
            '';
            meta = {
              description = "Native GPUI frontend for T3 Code";
              homepage = "https://github.com/conao3/rust-t3code-gpui";
              mainProgram = "t3code-gpui";
              platforms = lib.platforms.unix;
            };
          };

          devShells.default = pkgs.mkShell {
            packages =
              nativeTools
              ++ (with pkgs; [
                cargo
                clippy
                gnumake
                rust-analyzer
                rustc
                rustfmt
              ]);
            buildInputs = buildLibs;
            LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
          };

          treefmt.programs = {
            nixfmt.enable = true;
            rustfmt.enable = true;
          };
        };
    };
}
