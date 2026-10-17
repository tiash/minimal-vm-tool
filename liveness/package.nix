{ pkgs ? import <nixpkgs> {}, rustPlatform ? pkgs.rustPlatform, ... }:

let
  rootPath = ./../..;
in

rustPlatform.buildRustPackage {
  name = "minimal-vm-liveness-agent";
  src = ./empty;
  cargoRoot = ".";
  cargoLock = {
    lockFile = rootPath + /Cargo.lock;
    # simple-test-logging is a git dev-dependency; nixpkgs needs an explicit
    # output hash for git deps (Cargo.lock only stores the rev, not a
    # content hash). See the exec agent's package.nix for the bump recipe.
    outputHashes = {
      "simple-test-logging-0.1.0" = "sha256-uGXVE1US2PNXqiDKMth+fgNWyU2NhT6E1M5KY31Qn40=";
    };
  };
  cargoBuildFlags = ["--bin" "minimal-vm-liveness-agent"];
  cargoTestFlags = ["--bin" "minimal-vm-liveness-agent"];
  doCheck = false;
  postPatch = ''
    # make sure Cargo.lock isn't RO
    cat ${rootPath + /Cargo.lock} >Cargo.lock
    cat ${rootPath + /Cargo.toml} >Cargo.toml
    cp -r ${rootPath + /protocol} protocol
    cp -r ${rootPath + /agent} agent
    cp -r ${rootPath + /liveness} liveness
  '';
}
