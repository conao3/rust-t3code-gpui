{
  description = "Unofficial native GPUI frontend for T3 Code";

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
            cargoLock = {
              lockFile = ./Cargo.lock;
              outputHashes."pelago-0.1.0" = "15q4ppvwfxx6g9wkza74wzmq3pbkks07ihx12fi8nhl2gl3gs7za";
            };
            cargoBuildFlags = [
              "--package"
              "t3code-gpui"
            ];
            doCheck = false;
            nativeBuildInputs =
              nativeTools
              ++ [ pkgs.makeWrapper ]
              ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [ pkgs.copyDesktopItems ];
            desktopItems = lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              (pkgs.makeDesktopItem {
                name = "t3code-gpui";
                desktopName = "T3 Code GPUI (Unofficial)";
                comment = "Unofficial native frontend for T3 Code";
                exec = "t3code-gpui";
                icon = "t3code-gpui";
                categories = [ "Development" ];
              })
            ];
            buildInputs = buildLibs;
            postInstall = ''
              install -Dm644 LICENSE $out/share/doc/t3code-gpui/LICENSE
              install -Dm644 THIRD_PARTY_NOTICES.md $out/share/doc/t3code-gpui/THIRD_PARTY_NOTICES.md
              install -Dm644 vendor/accesskit_atspi_common/LICENSE-APACHE $out/share/doc/t3code-gpui/AccessKit-LICENSE-APACHE
              install -Dm644 vendor/accesskit_atspi_common/LICENSE-MIT $out/share/doc/t3code-gpui/AccessKit-LICENSE-MIT
              install -Dm644 vendor/accesskit_atspi_common/LICENSE.chromium $out/share/doc/t3code-gpui/AccessKit-LICENSE.chromium
              install -Dm644 crates/t3code-gpui/assets/Inter-LICENSE.txt $out/share/doc/t3code-gpui/Inter-LICENSE.txt
              install -m644 crates/t3code-gpui/assets/licenses/{RaTeX-MIT,KaTeX-fonts-NOTICE,SIL-OFL-1.1,Pelago-MIT}.txt $out/share/doc/t3code-gpui/
              install -Dm644 crates/t3code-gpui/assets/licenses/Unicode-3.0.txt $out/share/doc/t3code-gpui/Unicode-3.0.txt
              install -Dm644 crates/t3-terminal/assets/Ghostty-LICENSE $out/share/doc/t3code-gpui/Ghostty-LICENSE
              install -Dm644 crates/t3-terminal/assets/Ghostty-VERSION $out/share/doc/t3code-gpui/Ghostty-VERSION
              install -Dm644 crates/t3-terminal/assets/Wasmtime-LICENSE $out/share/doc/t3code-gpui/Wasmtime-LICENSE
            ''
            + lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              install -Dm644 assets/t3code-gpui.svg $out/share/icons/hicolor/scalable/apps/t3code-gpui.svg
            '';
            postFixup = lib.optionalString pkgs.stdenv.hostPlatform.isLinux ''
              wrapProgram $out/bin/t3code-gpui \
                --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs} \
                --prefix PATH : ${lib.makeBinPath [ pkgs.pipewire ]}
            '';
            meta = {
              license = lib.licenses.asl20;
              description = "Unofficial native GPUI frontend for T3 Code";
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
                ruff
                (python3.withPackages (
                  python:
                  [
                    python.websockets
                    python.pillow
                  ]
                  ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
                    python.pyatspi
                    python.pygobject3
                  ]
                ))
              ])
              ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
                pkgs.pipewire
                pkgs.at-spi2-core
                pkgs.gobject-introspection
                pkgs.dbus
                pkgs.xdotool
                pkgs.xorg-server
                pkgs.xfwm4
                pkgs.imagemagick
              ];
            buildInputs = buildLibs;
            LD_LIBRARY_PATH = lib.makeLibraryPath runtimeLibs;
            T3CODE_ATSPI_LAUNCHER = lib.optionalString pkgs.stdenv.hostPlatform.isLinux "${pkgs.at-spi2-core}/libexec/at-spi-bus-launcher";
            GI_TYPELIB_PATH = lib.optionalString pkgs.stdenv.hostPlatform.isLinux "${pkgs.at-spi2-core}/lib/girepository-1.0";
          };

          treefmt.programs = {
            nixfmt.enable = true;
            rustfmt.enable = true;
            ruff-format.enable = true;
          };
          treefmt.settings.global.excludes = [ "vendor/**" ];
        };
    };
}
