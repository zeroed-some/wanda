{ pkgs ? import <nixpkgs> {} }:

let
  # FHS environment for running Wine/Proton binaries
  fhs = pkgs.buildFHSEnv {
    name = "wanda-fhs";
    targetPkgs = pkgs: with pkgs; [
      # Wine and winetricks
      wineWowPackages.stable
      winetricks

      # 64-bit libraries Wine needs
      # NOTE: libGL and vulkan-loader are NOT included here - we use the system's
      # NVIDIA drivers via /run/opengl-driver to avoid Mesa/NVIDIA conflicts
      freetype
      fontconfig
      libpng
      libjpeg
      xorg.libX11
      xorg.libXcursor
      xorg.libXrandr
      xorg.libXi
      xorg.libXinerama
      xorg.libXext
      xorg.libXxf86vm
      xorg.libXrender
      xorg.libXcomposite
      xorg.libXfixes

      # Audio
      alsa-lib
      libpulseaudio

      # Networking
      openssl
      gnutls

      # Misc
      zlib
      ncurses
      libxkbcommon
    ];

    multiPkgs = pkgs: with pkgs; [
      # 32-bit libraries (multilib)
      # NOTE: libGL and vulkan-loader removed - using system NVIDIA drivers
      freetype
      fontconfig
      libpng
      libjpeg
      xorg.libX11
      xorg.libXcursor
      xorg.libXrandr
      xorg.libXi
      xorg.libXinerama
      xorg.libXext
      xorg.libXxf86vm
      xorg.libXrender
      xorg.libXcomposite
      xorg.libXfixes
      alsa-lib
      libpulseaudio
      openssl
      gnutls
      zlib
      ncurses
    ];

    runScript = "bash";

    profile = ''
      export WINEPREFIX="$HOME/.local/share/wanda/prefix"

      # Use system's NVIDIA drivers instead of Mesa to avoid driver conflicts
      # This is critical for stability - mixing Mesa userspace with NVIDIA kernel driver causes crashes
      export LD_LIBRARY_PATH="/run/opengl-driver/lib:/run/opengl-driver-32/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
      export __GLX_VENDOR_LIBRARY_NAME=nvidia
      export LIBVA_DRIVER_NAME=nvidia

      # Vulkan ICD - use NVIDIA's
      export VK_ICD_FILENAMES=/run/opengl-driver/share/vulkan/icd.d/nvidia_icd.x86_64.json:/run/opengl-driver-32/share/vulkan/icd.d/nvidia_icd.i686.json
    '';
  };
in
pkgs.mkShell {
  name = "wanda-dev";

  buildInputs = with pkgs; [
    # Rust toolchain
    rustc
    cargo
    rustfmt
    clippy
    rust-analyzer

    # Tauri system dependencies
    pkg-config
    gtk3
    webkitgtk_4_1
    libappindicator-gtk3
    librsvg

    # Additional Tauri deps
    openssl
    glib
    cairo
    pango
    gdk-pixbuf
    atk
    libsoup_3

    # Node.js for frontend
    nodejs_22
    nodePackages.npm

    # FHS environment for Wine/Proton
    fhs
  ];

  # Set up pkg-config path
  PKG_CONFIG_PATH = with pkgs; lib.makeSearchPath "lib/pkgconfig" [
    gtk3.dev
    webkitgtk_4_1.dev
    openssl.dev
    glib.dev
    cairo.dev
    pango.dev
    gdk-pixbuf.dev
    atk.dev
    libsoup_3.dev
  ];

  # Library paths for Tauri linking
  LD_LIBRARY_PATH = with pkgs; lib.makeLibraryPath [
    gtk3
    webkitgtk_4_1
    libappindicator-gtk3
    librsvg
    libsoup_3
    glib
    cairo
    pango
    gdk-pixbuf
    atk
    openssl
  ];

  shellHook = ''
    echo ""
    echo "=== WANDA Development Shell ==="
    echo ""
    echo "Build commands:"
    echo "  cargo build -p wanda-cli       # Build CLI"
    echo "  cargo build -p wanda-gui       # Build GUI"
    echo "  cargo build --release          # Release build"
    echo ""
    echo "Run WANDA:"
    echo "  ./target/release/wanda -v init # Initialize with verbose output"
    echo ""
    echo "For Wine/Proton operations (FHS environment):"
    echo "  wanda-fhs                      # Enter FHS shell for Wine"
    echo "  wanda-fhs -c 'wine --version'  # Run wine command"
    echo ""
    echo "GUI development:"
    echo "  cd crates/wanda-gui/frontend && npm install"
    echo "  cargo tauri dev"
    echo ""
  '';
}
