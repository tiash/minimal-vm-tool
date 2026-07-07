{ pkgs ? import <nixpkgs> {}, ... }:

let
  rootPath = ./..;
in

pkgs.rustPlatform.buildRustPackage {
  name = "minimal-vm-exec-agent";
  src = ./empty;
  cargoRoot = ".";
  cargoLock = {
    lockFile = rootPath + /Cargo.lock;
    # minimal-vm-exec-agent pulls simple-test-logging in as a git
    # dev-dependency; nixpkgs needs an explicit output hash for git deps
    # (Cargo.lock only stores the rev, not a content hash).
    outputHashes = {
      "simple-test-logging-0.1.0" = "sha256-CA0KkAPZ62vGqd9fyZupd/LazanZCY5B/z7TJwM3wYE=";
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
  '';
}
