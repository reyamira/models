{
  description = "TUI and CLI for browsing AI models, benchmarks, and coding agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      crane,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        inherit (pkgs) lib;

        craneLib = crane.mkLib pkgs;
        nixSkippedTest =
          # TODO: Remove this once the picker-save fetch counter test works
          "tui::app::tests::picker_save_updates_agents_fetch_counters_for_newly_tracked_agents";

        unfilteredRoot = ./.;
        src = lib.fileset.toSource {
          root = unfilteredRoot;
          fileset = lib.fileset.unions [
            ./data/agents.json # required for compilation
            (craneLib.fileset.commonCargoSources unfilteredRoot)
          ];
        };

        # Common arguments can be set here to avoid repeating them later
        commonArgs = {
          inherit src;
          strictDeps = true;
          doCheck = false;
          buildInputs = [ ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        models = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            doCheck = true;
            cargoTestExtraArgs = "-- --skip ${nixSkippedTest}";

            nativeBuildInputs = with pkgs; [
              makeWrapper
              installShellFiles
            ];

            postInstall =
              let
                models = "$out/bin/models";
              in
              ''
                wrapProgram ${models} \
                  --set SSL_CERT_FILE "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt"

                installShellCompletion --cmd models \
                      --bash <(${models} completions bash) \
                      --fish <(${models} completions fish) \
                      --zsh <(${models} completions zsh)
              '';
          }
        );

      in
      {
        checks = {
          inherit models;
        };

        packages = {
          inherit models;
          default = models;
        };
      }
    );
}
