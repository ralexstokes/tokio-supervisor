{
  description = "tokio-supervisor development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    crane = {
      url = "github:ipetkov/crane";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      crane,
      rust-overlay,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        nightlyToolchain = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [
            "clippy"
            "rustfmt"
          ];
        };
        craneLib = crane.mkLib pkgs;
        craneLibStable = craneLib.overrideToolchain rustToolchain;
        craneLibNightly = craneLib.overrideToolchain nightlyToolchain;
        hasCargoManifest = builtins.pathExists ./Cargo.toml;

        cargoChecks =
          if hasCargoManifest then
            let
              cargoSrc = craneLibStable.cleanCargoSource ./.;
              commonArgs = {
                src = cargoSrc;
                strictDeps = true;
              };
              cargoArtifacts = craneLibStable.buildDepsOnly commonArgs;
            in
            {
              cargo-fmt = craneLibNightly.cargoFmt {
                src = cargoSrc;
                cargoExtraArgs = "--all";
              };

              cargo-clippy = craneLibNightly.cargoClippy (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  cargoExtraArgs = "--locked";
                  cargoClippyExtraArgs = "--workspace --all-targets --all-features -- -D warnings";
                }
              );

              cargo-build = craneLibStable.cargoBuild (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  cargoExtraArgs = "--locked --workspace --all-targets --all-features";
                }
              );

              cargo-test = craneLibStable.cargoTest (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  cargoExtraArgs = "--locked --workspace --all-targets --all-features";
                }
              );
            }
          else
            { };
      in
      {
        formatter = pkgs.nixfmt;

        checks = {
          nixfmt = pkgs.runCommandLocal "nixfmt-check" { nativeBuildInputs = [ pkgs.nixfmt ]; } ''
            nixfmt --check ${./flake.nix}
            touch $out
          '';
        }
        // cargoChecks;

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            git
            just
            mdbook
            ripgrep
            rustup
            rustToolchain
            nightlyToolchain
          ];
        };
      }
    );
}
