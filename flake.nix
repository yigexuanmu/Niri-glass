{
  description = "Niri-glass: niri (SHORiN-KiWATA fork) with a liquid-glass / refraction background effect";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    # Raw SHORiN-KiWATA/niri source (no flake). We build it ourselves with the
    # community niri-package expression (same approach as yigexuanmu/shorin-niri-nix)
    # and apply the liquid-glass overlay on top.
    shorin-niri = {
      url = "github:SHORiN-KiWATA/niri";
      flake = false;
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      shorin-niri,
    }:
    let
      revision = self.shortRev or self.dirtyShortRev or "unknown";

      niri-package =
        {
          lib,
          cairo,
          dbus,
          libGL,
          # niri's libdisplay-info-sys requires libdisplay-info < 0.4.0; nixpkgs
          # default is now 0.4.x, so use the pinned 0.3 attribute.
          libdisplay-info_0_3,
          libinput,
          seatd,
          libxkbcommon,
          libgbm,
          pango,
          pipewire,
          pkg-config,
          rustPlatform,
          systemd,
          wayland,
          installShellFiles,
          withDbus ? true,
          withSystemd ? true,
          withScreencastSupport ? true,
          withDinit ? false,
        }:

        rustPlatform.buildRustPackage {
          pname = "niri-glass";
          version = revision;

          src = shorin-niri;

          postPatch = ''
            patchShebangs resources/niri-session
            substituteInPlace resources/niri.service \
              --replace-fail 'ExecStart=niri' "ExecStart=$out/bin/niri"
            echo "==> Applying Niri-glass grid-overview config patch"
            patch -p1 < ${./patches/grid-overview-config.patch}
          ''
          + ''
            echo "==> Applying Niri-glass liquid-glass overlay"
            chmod -R u+w src/render_helpers niri-config/src
            cp --no-preserve=mode ${./src/render_helpers/liquid_glass.rs}              src/render_helpers/liquid_glass.rs
            cp --no-preserve=mode ${./src/render_helpers/background_effect.rs}         src/render_helpers/background_effect.rs
            cp --no-preserve=mode ${./src/render_helpers/framebuffer_effect.rs}        src/render_helpers/framebuffer_effect.rs
            cp --no-preserve=mode ${./src/render_helpers/xray.rs}                      src/render_helpers/xray.rs
            cp --no-preserve=mode ${./src/render_helpers/mod.rs}                       src/render_helpers/mod.rs
            cp --no-preserve=mode ${./src/render_helpers/shaders/clipped_surface.frag} src/render_helpers/shaders/clipped_surface.frag
            cp --no-preserve=mode ${./src/render_helpers/shaders/mod.rs}               src/render_helpers/shaders/mod.rs
            cp --no-preserve=mode ${./niri-config/src/appearance.rs}                   niri-config/src/appearance.rs
          '';

          cargoLock = {
            allowBuiltinFetchGit = true;
            lockFile = "${shorin-niri}/Cargo.lock";
          };

          strictDeps = true;

          nativeBuildInputs = [
            rustPlatform.bindgenHook
            pkg-config
            installShellFiles
          ];

          buildInputs = [
            cairo
            dbus
            libGL
            libdisplay-info_0_3
            libinput
            seatd
            libxkbcommon
            libgbm
            pango
            wayland
          ]
          ++ lib.optional (withDbus || withScreencastSupport || withSystemd) dbus
          ++ lib.optional withScreencastSupport pipewire
          ++ lib.optional withSystemd systemd;

          buildFeatures =
            lib.optional withDbus "dbus"
            ++ lib.optional withDinit "dinit"
            ++ lib.optional withScreencastSupport "xdp-gnome-screencast"
            ++ lib.optional withSystemd "systemd";
          buildNoDefaultFeatures = true;

          preCheck = ''
            export XDG_RUNTIME_DIR="$(mktemp -d)"
          '';

          checkFlags = [
            "--skip=::egl"
            "--skip=tests::animations"
          ];

          postInstall = ''
            installShellCompletion --cmd niri \
              --bash <($out/bin/niri completions bash) \
              --fish <($out/bin/niri completions fish) \
              --nushell <($out/bin/niri completions nushell) \
              --zsh <($out/bin/niri completions zsh)

            install -Dm644 resources/niri.desktop -t $out/share/wayland-sessions
            install -Dm644 resources/niri-portals.conf -t $out/share/xdg-desktop-portal
          ''
          + lib.optionalString withSystemd ''
            install -Dm755 resources/niri-session $out/bin/niri-session
            install -Dm644 resources/niri{.service,-shutdown.target} -t $out/lib/systemd/user
          '';

          env = {
            # Force linking with libEGL and libwayland-client so they end up in RPATH and
            # can be discovered by `dlopen()`
            RUSTFLAGS = toString (
              map (arg: "-C link-arg=" + arg) [
                "-Wl,--push-state,--no-as-needed"
                "-lEGL"
                "-lwayland-client"
                "-Wl,--pop-state"
              ]
            );
            NIRI_BUILD_COMMIT = revision;
          };

          passthru = {
            providedSessions = [ "niri" ];
          };

          meta = {
            description = "niri (SHORiN-KiWATA fork) with a liquid-glass / refraction background effect";
            homepage = "https://github.com/yigexuanmu/Niri-glass";
            license = lib.licenses.gpl3Only;
            mainProgram = "niri";
            platforms = lib.platforms.linux;
          };
        };

      inherit (nixpkgs) lib;
      systems = lib.intersectLists lib.systems.flakeExposed lib.platforms.linux;

      forAllSystems = lib.genAttrs systems;
      nixpkgsFor = forAllSystems (system: nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (
        system:
        let
          niri-glass = nixpkgsFor.${system}.callPackage niri-package { };
        in
        {
          inherit niri-glass;

          # NOTE: For development purposes only.
          niri-glass-debug = niri-glass.overrideAttrs (
            newAttrs: oldAttrs: {
              pname = oldAttrs.pname + "-debug";

              cargoBuildType = "debug";
              cargoCheckType = newAttrs.cargoBuildType;

              dontStrip = true;
            }
          );

          default = niri-glass;
        }
      );

      checks = forAllSystems (system: {
        inherit (self.packages.${system}) niri-glass-debug;
      });

      # `nix run` / `nix run .#niri-session`
      apps = forAllSystems (
        system:
        let
          pkgs = self.packages.${system}.niri-glass;
        in
        {
          default = {
            type = "app";
            program = "${pkgs}/bin/niri";
            meta.description = "Run the niri-glass compositor";
          };
          niri-session = {
            type = "app";
            program = "${pkgs}/bin/niri-session";
            meta.description = "Run niri-glass as a systemd user session";
          };
        }
      );

      # `nix develop` — full niri build environment for hacking on the overlay.
      # NOTE: this repo only contains overlay files (no Cargo.toml). To build,
      # apply the overlay onto a checkout of SHORiN-KiWATA/niri (see ./install.sh)
      # and run `cargo build --release` from there inside this shell.
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgsFor.${system};
          niriGlass = self.packages.${system}.niri-glass;
          rustfmt' = pkgs.rustfmt.override { asNightly = true; };
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ niriGlass ];
            packages = builtins.attrValues {
              inherit (pkgs)
                rustc
                cargo
                clippy
                cargo-insta
                ;
              inherit rustfmt';
            };
            # Required for `dlopen()` of libEGL / libwayland-client; see the
            # package expression. Do not overwrite, only append.
            RUSTFLAGS = niriGlass.RUSTFLAGS or "";
            shellHook = ''
              echo "niri-glass dev shell — Rust + niri build deps ready."
              echo "This repo is an overlay; build against a SHORiN-KiWATA/niri checkout."
            '';
          };
        }
      );

      overlays.default = final: _prev: {
        niri-glass = final.callPackage niri-package { };
      };

      nixosModules.default = import ./nix/nixos-module.nix self;
      homeManagerModules.default = import ./nix/home-manager-module.nix self;

      formatter = forAllSystems (system: nixpkgsFor.${system}.nixfmt-rfc-style);
    };
}
