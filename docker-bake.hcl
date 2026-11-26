# Build the compiler outputs used in the browser package workflow.
# Each recipe that artifacts/npm/sources.json names, by the name of its folder.
variable "RECIPES" {
  default = ["speex", "bergamot", "rnnoise"]
}

# "gha" in CI: each recipe then reads and writes a GitHub Actions cache of
# its own. It keeps only the outputs (mode=min, the default), a few kilobytes
# to a few megabytes, which is enough for a recipe that did not change to be
# taken whole from the cache without its toolchain even being pulled.
# mode=max would also keep every layer of every step, a toolchain image of a
# gigabyte or more among them, in a repository cache that the image job
# already fills.
variable "RECIPE_CACHE" {
  default = ""
}

group "recipes" {
  targets = ["recipe"]
}

target "recipe" {
  name       = "recipe-${name}"
  matrix     = { name = RECIPES }
  dockerfile = "artifacts/recipes/Dockerfile"
  context    = "."
  target     = "recipe-${name}"
  output     = ["type=local,dest=artifacts/recipes/${name}/out"]
  # Only the outputs belong in out/; buildx in CI would add provenance.json.
  attest     = ["type=provenance,disabled=true"]
  cache-from = RECIPE_CACHE == "gha" ? ["type=gha,scope=recipe-${name}"] : []
  cache-to   = RECIPE_CACHE == "gha" ? ["type=gha,scope=recipe-${name}"] : []
}
