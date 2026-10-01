import { describe, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

interface Step {
  name: string;
  image: string;
  script: string;
  volumeMounts?: { name: string; mountPath: string; readOnly?: boolean }[];
  env?: { name: string; value: string }[];
  envFrom?: { secretRef: { name: string } }[];
}
interface Task {
  name: string;
  runAfter?: string[];
  params?: { name: string; value: string }[];
  taskSpec: {
    steps: Step[];
    volumes?: { name: string; secret?: { secretName: string }; emptyDir?: Record<string, never> }[];
  };
}
interface Pipeline {
  apiVersion: string;
  kind: string;
  metadata: { annotations: Record<string, string> };
  spec: { pipelineSpec: { tasks: Task[] }; taskRunSpecs: { pipelineTaskName: string; podTemplate: { nodeSelector: Record<string, string> } }[] };
}

const root = join(import.meta.dir, "..");
const yaml = readFileSync(join(import.meta.dir, "smarthome-mcp-release.yaml"), "utf8");
const pipeline = Bun.YAML.parse(yaml) as Pipeline;
const tasks = pipeline.spec.pipelineSpec.tasks;
const task = (name: string): Task => {
  const found = tasks.find((item) => item.name === name);
  if (!found) throw new Error(`missing task: ${name}`);
  return found;
};
const revision = "a".repeat(40);
const digest = `sha256:${"b".repeat(64)}`;
const image = `cr.holdenitdown.net/rfhold/smarthome-mcp@${digest}`;

function configFixture(scenario: string, arch: string) {
  const config = { os: "linux", architecture: arch, config: { User: "65532:65532", Labels: { "org.opencontainers.image.revision": revision } } };
  const unrelated = scenario.startsWith("nested-") ? structuredClone(config) : undefined;
  const defect = scenario.replace(/^nested-/, "");
  if (defect === "wrong-os") config.os = "windows";
  if (defect === "wrong-architecture") config.architecture = "wrong";
  if (defect === "root") config.config.User = "0";
  if (defect === "wrong-revision") config.config.Labels["org.opencontainers.image.revision"] = "wrong";
  const json = JSON.stringify({ ...config, unrelated });
  return json + (scenario === "malformed-config" ? " invalid" : scenario === "multiple-config" ? json : "");
}

function manifestFixture(scenario: string) {
  const manifest: any = { manifests: ["amd64", "arm64"].map((architecture) => ({ platform: { architecture, os: "linux" } })) };
  if (scenario === "single") return '{"architecture":"amd64"}';
  if (scenario === "missing-arm64") manifest.manifests.pop();
  if (scenario === "nested-platform") {
    manifest.unrelated = structuredClone(manifest);
    manifest.manifests = [];
  }
  if (scenario === "wrong-platform-os") for (const entry of manifest.manifests) entry.platform.os = "windows";
  if (scenario === "manifest-not-array") manifest.manifests = { platform: { os: "linux", architecture: "amd64" } };
  const json = JSON.stringify(manifest);
  return json + (scenario === "malformed-manifest" ? " invalid" : scenario === "multiple-manifest" ? json : "");
}

describe("release declaration", () => {
  test("stages pinned crane into the pinned jq execution image on amd64", () => {
    const promotion = task("promote-release");
    expect(promotion.taskSpec.volumes).toEqual([{ name: "crane-tool", emptyDir: {} }]);
    const [stage, verify] = promotion.taskSpec.steps;
    expect(stage.name).toBe("stage-crane");
    expect(stage.image).toBe("gcr.io/go-containerregistry/crane:debug@sha256:54b27703e6c602fbd6f95712910e9c8d45d4361a59274bde38aeec943734e424");
    expect(stage.script).toContain("cp /ko-app/crane /tools/crane");
    expect(stage.volumeMounts).toEqual([{ name: "crane-tool", mountPath: "/tools" }]);
    expect(verify.image).toBe("cr.holdenitdown.net/rfhold/general-ci@sha256:6943f91d774980357b24730a14ac4b026325d50962df4b2e15e24c3f43190e5b");
    expect(verify.volumeMounts).toEqual([{ name: "crane-tool", mountPath: "/tools", readOnly: true }]);
    expect(verify.script).toContain('export PATH="/tools:$PATH"');
    expect(verify.script).toContain("crane version");
    expect(pipeline.spec.taskRunSpecs.find((entry) => entry.pipelineTaskName === "promote-release")?.podTemplate.nodeSelector).toEqual({ "kubernetes.io/arch": "amd64" });
  });
  test("all inline shell scripts parse", () => {
    for (const item of tasks) for (const step of item.taskSpec.steps) {
      expect(Bun.spawnSync(["/bin/sh", "-n", "-c", step.script]).exitCode).toBe(0);
    }
  });
  test("stable tag routing and policy gates precede immutable promotion", () => {
    expect(pipeline.apiVersion).toBe("tekton.dev/v1");
    expect(pipeline.kind).toBe("PipelineRun");
    const cel = pipeline.metadata.annotations["pipelinesascode.tekton.dev/on-cel-expression"];
    expect(cel).toContain('event == "push"');
    expect(cel).toContain("^refs/tags/v(0|[1-9][0-9]*)");
    expect(tasks.map((item) => item.name)).toEqual([
      "fetch-repository", "validate-release", "scan-private-material",
      "promote-release", "deploy-production",
    ]);
    expect(task("promote-release").runAfter).toEqual(["validate-release", "scan-private-material"]);
    const policy = task("validate-release");
    expect(policy.runAfter).toEqual(["fetch-repository"]);
    expect(policy.taskSpec.volumes?.[0].secret?.secretName).toBe("smarthome-mcp-release-trusted-signers");
    const signature = policy.taskSpec.steps[0].script;
    for (const gate of [
      'git cat-file -t "refs/tags/$tag"', "signing-key.asc",
      "gpg.format=openpgp verify-tag", 'refs/tags/$tag^{commit}',
      'git rev-parse HEAD', 'git merge-base --is-ancestor "$REVISION" refs/remotes/origin/main',
    ]) expect(signature).toContain(gate);
    expect(policy.taskSpec.steps[1].script).toContain("cargo.package.version, python.project.version, component.version");
    expect(task("scan-private-material").taskSpec.steps[0].script).toContain('test "$status" -eq 1');
    expect(yaml).not.toMatch(/buildctl|buildkit|docker build|HOME_ASSISTANT_TOKEN|HOME_ASSISTANT_SSH_PASSWORD|cp \/etc\/git-auth/);
  });

  test("prod deploy consumes promotion result with normal credentials only", () => {
    const deploy = task("deploy-production");
    expect(deploy.runAfter).toEqual(["promote-release"]);
    expect(deploy.params).toEqual([{ name: "image", value: "$(tasks.promote-release.results.image)" }]);
    expect(deploy.taskSpec.steps[0].envFrom).toEqual([
      { secretRef: { name: "pulumi-credentials" } },
      { secretRef: { name: "authentik-credentials" } },
    ]);
    expect(deploy.taskSpec.volumes?.[0].secret?.secretName).toBe("tekton-cluster-kubeconfig");
    expect(deploy.taskSpec.steps[0].script).toContain('pulumi preview --stack prod --diff --config "image=$APP_IMAGE"');
    expect(deploy.taskSpec.steps[0].script).toContain('pulumi up --stack prod --yes --skip-preview --config "image=$APP_IMAGE"');
  });

  test("confirmed prod SSH settings preserve narrower auth and storage policy", () => {
    const prod = Bun.YAML.parse(readFileSync(join(root, "infra/pulumi/Pulumi.prod.yaml"), "utf8")) as { config: Record<string, unknown> };
    const expected = {
      homeAssistantSshHost: "172.16.1.10", homeAssistantSshPort: "2200",
      homeAssistantSshUsername: "root", homeAssistantSshConfigRoot: "/homeassistant",
      homeAssistantSshEgressCidr: "172.16.1.10/32",
      protectData: "true", databaseStorageSize: "20Gi", backupRetention: "30d",
    };
    for (const [key, value] of Object.entries(expected)) expect(prod.config[`smarthome-mcp:${key}`]).toBe(value);
    expect(prod.config["smarthome-mcp:mcpOAuthCimdTrustedPrivateOrigins"]).toBeUndefined();
  });
});

describe("version gate execution", () => {
  const versionScript = task("validate-release").taskSpec.steps[1].script
    .replace('test "$(bun --version)" = "1.3.5"', ":");
  for (const [tag, componentVersion, succeeds] of [
    ["v0.2.0", "0.2.0", true],
    ["v0.2.0", "0.1.0", false],
    ["v0.3.0", "0.2.0", false],
    ["v00.2.0", "0.2.0", false],
    ["v0.2.0-rc.1", "0.2.0", false],
    ["v0.2.0+build", "0.2.0", false],
  ] as const) test(`${tag} / component ${componentVersion}`, () => {
    const directory = mkdtempSync(join(tmpdir(), "smarthome-release-version-"));
    try {
      writeFileSync(join(directory, "Cargo.toml"), '[package]\nversion = "0.2.0"\n');
      writeFileSync(join(directory, "pyproject.toml"), '[project]\nversion = "0.2.0"\n');
      const script = versionScript.replace("custom_components/smarthome_mcp/manifest.json", "manifest.json");
      writeFileSync(join(directory, "manifest.json"), JSON.stringify({ version: componentVersion }));
      const resultPath = join(directory, "tag");
      const result = Bun.spawnSync(["/bin/sh", "-c", script], {
        cwd: directory,
        env: { ...process.env, SOURCE_BRANCH: `refs/tags/${tag}`, TAG_RESULT: resultPath },
      });
      expect(result.exitCode === 0).toBe(succeeds);
      if (succeeds) expect(readFileSync(resultPath, "utf8")).toBe(tag);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
});

function promote(scenario: string): { status: number; calls: string; result: string } {
  const directory = mkdtempSync(join(tmpdir(), "smarthome-release-test-"));
  try {
    writeFileSync(join(directory, "crane"), `#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$CALLS"
case "$1" in
  version) [ "$SCENARIO" != crane-unavailable ] || exit 1; printf 'fixture crane\n' ;;
  digest)
    case "$2" in
      *:preview-*)
        if [ "$SCENARIO" = preview-missing ]; then echo MANIFEST_UNKNOWN >&2; exit 1; fi
        if [ "$SCENARIO" = malformed ]; then echo sha256:bad; else echo "$DIGEST"; fi ;;
      *)
        if [ -f "$COPIED" ]; then echo "$DIGEST"; exit; fi
        case "$SCENARIO" in
          absent) echo 'MANIFEST_UNKNOWN: manifest unknown' >&2; exit 1 ;;
          conflict) echo sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc ;;
          unauthorized) echo 'UNAUTHORIZED: authentication required' >&2; exit 1 ;;
          timeout) echo 'dial tcp: i/o timeout' >&2; exit 1 ;;
          notfound) echo '404 Not Found' >&2; exit 1 ;;
          nameunknown) echo 'NAME_UNKNOWN: repository unknown' >&2; exit 1 ;;
          *) echo "$DIGEST" ;;
        esac ;;
    esac ;;
  manifest)
    printf '%s' "$MANIFEST_JSON" ;;
  config)
    case "$3" in
      linux/amd64) printf '%s' "$CONFIG_AMD64" ;;
      linux/arm64) printf '%s' "$CONFIG_ARM64" ;;
      *) exit 1 ;;
    esac ;;
  copy) touch "$COPIED" ;;
  *) exit 1 ;;
esac
`, { mode: 0o755 });
    const resultPath = join(directory, "image");
    const callsPath = join(directory, "calls");
    const script = task("promote-release").taskSpec.steps[1].script.replace("$(results.image.path)", resultPath);
    const result = Bun.spawnSync(["/bin/sh", "-c", script], {
      env: { ...process.env, PATH: `${directory}:${process.env.PATH}`, SCENARIO: scenario,
        REVISION: revision, RELEASE_TAG: "v0.2.0", DIGEST: digest,
        MANIFEST_JSON: manifestFixture(scenario), CONFIG_AMD64: configFixture(scenario, "amd64"), CONFIG_ARM64: configFixture(scenario, "arm64"),
        CALLS: callsPath, COPIED: join(directory, "copied") },
    });
    const calls = readFileSync(callsPath, "utf8");
    let output = "";
    if (existsSync(resultPath)) output = readFileSync(resultPath, "utf8");
    return { status: result.exitCode, calls, result: output };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe("promotion shell regression with local registry stub", () => {
  for (const scenario of ["absent", "same"]) test(scenario, () => {
    const result = promote(scenario);
    expect(result.status).toBe(0);
    expect(result.result).toBe(image);
    expect(result.calls).toContain(`config --platform linux/amd64 ${image}`);
    expect(result.calls).toContain(`config --platform linux/arm64 ${image}`);
    if (scenario === "absent") expect(result.calls).toContain(`copy ${image} cr.holdenitdown.net/rfhold/smarthome-mcp:v0.2.0`);
    else expect(result.calls).not.toContain("copy ");
  });
  for (const scenario of ["crane-unavailable", "preview-missing", "malformed", "single", "missing-arm64", "root", "wrong-revision", "wrong-os", "wrong-architecture", "nested-root", "nested-wrong-revision", "nested-wrong-os", "nested-wrong-architecture", "malformed-config", "multiple-config", "nested-platform", "wrong-platform-os", "manifest-not-array", "malformed-manifest", "multiple-manifest", "conflict", "unauthorized", "timeout", "notfound", "nameunknown"]) test(scenario, () => {
    const result = promote(scenario);
    expect(result.status).not.toBe(0);
    expect(result.calls).not.toContain("copy ");
    expect(result.result).toBe("");
  });
});
