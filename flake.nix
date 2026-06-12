{
  description = "TUI and CLI for browsing AI models, benchmarks, and coding agents";

  # CI pushes builds to this cache; users who accept the prompt download
  # prebuilt binaries instead of compiling. The key is the cache's public
  # signing key (verification only — safe to commit).
  nixConfig = {
    extra-substituters = [ "https://modelsdev.cachix.org" ];
    extra-trusted-public-keys = [
      "modelsdev.cachix.org-1:P/sJsc6wE55M7DWEGL7SjWAxKTD8TjZMYM8Iows77Ls="
    ];
  };

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
        unfilteredRoot = ./.;
        src = lib.fileset.toSource {
          root = unfilteredRoot;
          fileset = lib.fileset.unions [
            ./data/agents.json # required for compilation
            ./data/v2 # committed source files read by drift-guard tests
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
