{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    let
      mkNes = pkgs:
        let
          toolchain = pkgs.rust-bin.stable.latest.default.override {
            targets = [ "wasm32-unknown-unknown" ];
          };
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
        in
        rec {
          nes = rustPlatform.buildRustPackage {
            pname = "nes";
            version = "0.1.0";
            inherit src cargoLock;
            cargoBuildFlags = [ "-p" "nes-core" ];
            cargoTestFlags = [ "-p" "nes-core" ];
          };

          web = rustPlatform.buildRustPackage {
            pname = "nes-web";
            version = "0.1.0";
            inherit src cargoLock;
            doCheck = false;
            buildPhase = ''
              runHook preBuild
              cargo build --release --offline -p nes-wasm --target wasm32-unknown-unknown
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              mkdir -p $out
              cp target/wasm32-unknown-unknown/release/nes_wasm.wasm $out/nes.wasm
              cp web/index.html web/nes-player.js $out/
              runHook postInstall
            '';
          };

          agent = rustPlatform.buildRustPackage {
            pname = "nes-agent";
            version = "0.1.0";
            inherit src cargoLock;
            cargoBuildFlags = [ "-p" "nes-agent" ];
            cargoTestFlags = [ "-p" "nes-agent" ];
          };

          native = rustPlatform.buildRustPackage {
            pname = "nes-native";
            version = "0.1.0";
            inherit src cargoLock;
            cargoBuildFlags = [ "-p" "nes-native" ];
            cargoTestFlags = [ "-p" "nes-native" ];
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.SDL2 ];
          };

          bundleWeb = { rom, name ? "nes-web-bundle" }:
            pkgs.runCommand name { } ''
              mkdir -p $out
              cp ${web}/nes.wasm $out/nes.wasm
              cp ${./web/nes-player.js} $out/nes-player.js
              cp ${rom} $out/rom.nes
            '';

          bundleNative = { rom, name ? "nes-native-bundle" }:
            pkgs.runCommand name { } ''
              mkdir -p $out/bin $out/share/${name}
              cp ${rom} $out/share/${name}/rom.nes
              cat > $out/bin/${name} <<WRAP
              #!${pkgs.runtimeShell}
              exec ${native}/bin/nes-native $out/share/${name}/rom.nes
              WRAP
              chmod +x $out/bin/${name}
            '';

          checkSuite = rustPlatform.buildRustPackage {
            pname = "nes-checks";
            version = "0.1.0";
            inherit src cargoLock;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.SDL2 ];
            doCheck = false;
            buildPhase = ''
              runHook preBuild
              cargo fmt --check
              cargo clippy --workspace --all-targets --offline -- -D warnings
              cargo test -p nes-core -p nes-agent --offline
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              touch $out
              runHook postInstall
            '';
          };

          test = pkgs.writeShellApplication {
            name = "nes-test";
            runtimeInputs = [ toolchain pkgs.pkg-config pkgs.stdenv.cc ];
            text = ''
              export PKG_CONFIG_PATH=${pkgs.SDL2.dev}/lib/pkgconfig''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}
              cargo fmt --check
              cargo clippy --workspace --all-targets -- -D warnings
              cargo test -p nes-core -p nes-agent
            '';
          };

          devShell = pkgs.mkShell {
            nativeBuildInputs = [ toolchain pkgs.pkg-config ];
            buildInputs = [ pkgs.SDL2 ];
          };
        };
    in
    {
      overlays.default = nixpkgs.lib.composeManyExtensions [
        rust-overlay.overlays.default
        (final: prev: { nes = mkNes final; })
      ];
    } // flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        built = mkNes pkgs;
      in
      {
        packages = {
          inherit (built) nes web native agent;
          default = built.nes;
        };

        lib = {
          inherit (built) bundleWeb bundleNative;
        };

        checks.test = built.checkSuite;

        apps.test = {
          type = "app";
          program = "${built.test}/bin/nes-test";
        };

        devShells.default = built.devShell;
      });
}
