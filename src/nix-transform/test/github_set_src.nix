{
  pkgs ? import <nixpkgs> { },
}:

pkgs.mkDerivation {

  src = fetchFromGitHub {
    owner = "t4ccer";
    repo = "cgt-tools";
    rev = "v0.7.0"; # foo
    hash = "";
    # a comment
  };

}
