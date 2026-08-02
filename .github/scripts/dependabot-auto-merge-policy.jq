def previous_version:
  (
    try (
      tostring
      | sub("^v"; "")
      | capture("^(?<major>[0-9]+)[.](?<minor>[0-9]+)(?:[.]|$)")
      | {
          major: (.major | tonumber),
          minor: (.minor | tonumber)
        }
    ) catch null
  ) // null;

type == "array" and length > 0 and
all(.[];
  (.prevVersion | previous_version) as $previous |
  (.packageEcosystem == "cargo" or .packageEcosystem == "github-actions") and
  .dependencyName != "dependabot/fetch-metadata" and
  .dependencyName != "actions/create-github-app-token" and
  .dependencyName != "release-plz/action" and
  .dependencyName != "rust-lang/crates-io-auth-action" and
  .maintainerChanges == false and
  (
    (
      .updateType == "version-update:semver-patch" and
      $previous != null and
      (
        $previous.major >= 1 or
        (
          .packageEcosystem == "cargo" and
          $previous.major == 0 and
          $previous.minor >= 1
        )
      )
    ) or
    (
      .updateType == "version-update:semver-minor" and
      $previous != null and
      $previous.major >= 1
    )
  )
)
