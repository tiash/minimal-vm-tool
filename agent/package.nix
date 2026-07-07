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
