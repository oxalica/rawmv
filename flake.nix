{
  description = "mv(1) but without cp(1) fallback. Simple wrapper of renameat2(2)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs = { self, nixpkgs }: {
    packages = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed (system: rec {
      default = rawmv;
      rawmv = nixpkgs.legacyPackages.${system}.rustPlatform.buildRustPackage {
        pname = "rawmv";
        inherit ((builtins.fromTOML (builtins.readFile (self + "/Cargo.toml"))).package) version;
        src = self;
        cargoLock.lockFile = self + "/Cargo.lock";
      };
    });
  };
}
