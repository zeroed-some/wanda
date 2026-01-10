{
  description = "WANDA - WeMod launcher for Linux";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Rust toolchain
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        # Common build inputs for Tauri
        tauriBuildInputs = with pkgs; [
          pkg-config
          gtk3
          webkitgtk_4_1
          libappindicator-gtk3
          librsvg
          openssl
          glib
          cairo
          pango
          gdk-pixbuf
          atk
          libsoup_3
        ];

        # Development tools
        devTools = with pkgs; [
          rustToolchain
          nodejs_20
          nodePackages.npm
          cargo-tauri
        ];

      in {
        devShells.default = pkgs.mkShell {
          buildInputs = tauriBuildInputs ++ devTools;

          PKG_CONFIG_PATH = with pkgs; lib.makeSearchPath "lib/pkgconfig" [
            gtk3.dev
            webkitgtk_4_1.dev
            libappindicator-gtk3.dev
            openssl.dev
            glib.dev
            cairo.dev
            pango.dev
            gdk-pixbuf.dev
            atk.dev
            libsoup_3.dev
          ];

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

          shellHook = ''
            echo "🪄 WANDA development environment"
            echo ""
            echo "Commands:"
            echo "  cargo build -p wanda-cli     # Build CLI only"
            echo "  cargo build -p wanda-gui     # Build GUI"
            echo "  cargo tauri dev              # Run GUI in dev mode"
            echo ""
          '';
        };

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "wanda-cli";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          buildInputs = tauriBuildInputs;
          nativeBuildInputs = [ pkgs.pkg-config ];

          # Only build CLI for now
          cargoBuildFlags = [ "-p" "wanda-cli" ];
        };
      }
    );
}
