{ pkgs ? import <nixpkgs> {}, rustPlatform ? pkgs.rustPlatform, ... }:

let
  rootPath = ./..;
in

rustPlatform.buildRustPackage {
  name = "minimal-vm-exec-agent";
  src = ./empty;
  cargoRoot = ".";
  cargoLock = {
    lockFile = rootPath + /Cargo.lock;
    # minimal-vm-exec-agent pulls simple-test-logging in as a git
    # dev-dependency; nixpkgs needs an explicit output hash for git deps
    # (Cargo.lock only stores the rev, not a content hash).
    outputHashes = {
      "simple-test-logging-0.1.0" = "sha256-uGXVE1US2PNXqiDKMth+fgNWyU2NhT6E1M5KY31Qn40=";
    };
  };
  cargoBuildFlags = ["--bin" "minimal-vm-exec-agent"];
  cargoTestFlags = ["--bin" "minimal-vm-exec-agent"];
  doCheck = false;
  postPatch = ''
    # make sure Cargo.lock isn't RO
    cat ${rootPath + /Cargo.lock} >Cargo.lock
    cp ${rootPath + /Cargo.toml} Cargo.toml
    cp -r ${rootPath + /agent} agent
    cp -r ${rootPath + /protocol} protocol
    cp -r ${rootPath + /liveness} liveness
  '';
}
