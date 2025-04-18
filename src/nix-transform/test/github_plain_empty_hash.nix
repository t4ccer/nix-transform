{
  pkgs ? import <nixpkgs> { },
}:

fetchFromGitHub {
  owner = "t4ccer";
  repo = "cgt-tools";
  rev = "v0.7.0";
  hash = "";
}
