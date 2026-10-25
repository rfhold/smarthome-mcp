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
  taskRef?: { name: string };
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
  const manifest: any = { schemaVersion: 2, mediaType: "application/vnd.oci.image.index.v1+json", manifests: ["amd64", "arm64"].map((architecture) => descriptor(scenario, architecture)) };
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

const sharedTasks = process.env.SHARED_TASKS_DIR ?? join(import.meta.dir, "../../homelab/src/components/tekton-tasks");
const sharedPromotion = (Bun.YAML.parse(readFileSync(join(sharedTasks, "service-promote-v1.yaml"), "utf8")) as any).spec.steps[0].script;
function hash(text: string) { return `sha256:${new Bun.CryptoHasher("sha256").update(text).digest("hex")}`; }
const configType = "application/vnd.oci.image.config.v1+json";
const manifestType = "application/vnd.oci.image.manifest.v1+json";
function platformFixture(scenario: string, arch: string) {
  const config = configFixture(scenario, arch);
  return JSON.stringify({ schemaVersion: 2, mediaType: manifestType, config: { mediaType: configType, digest: hash(config), size: Buffer.byteLength(config) }, layers: [] });
}
function descriptor(scenario: string, architecture: string) {
  const manifest = platformFixture(scenario, architecture);
  return { mediaType: manifestType, digest: hash(manifest), size: Buffer.byteLength(manifest), platform: { architecture, os: "linux" } };
}

const digest = hash(manifestFixture("absent"));
const image = `cr.holdenitdown.net/rfhold/smarthome-mcp@${digest}`;

function promote(scenario: string, overrides: Record<string, string> = {}) {
  const directory = mkdtempSync(join(tmpdir(), "shared-release-test-"));
  try {
    const output = join(directory, "images");
    const calls = join(directory, "calls");
    const scriptPath = join(directory, "promote.ts");
    const repository = JSON.parse(task("promote-release").params!.find((p: any) => p.name === "images").value)[0].repository;
    const objects: Record<string, string> = {};
    for (const arch of ["amd64", "arm64"]) {
      const config = configFixture(scenario, arch), manifest = platformFixture(scenario, arch);
      objects[`blobs/${hash(config)}`] = config;
      objects[`manifests/${hash(manifest)}`] = manifest;
    }
    const index = manifestFixture(scenario);
    objects[`manifests/${revision}`] = index;
    const harness = `
import { appendFileSync } from "node:fs";
const objects = ${JSON.stringify(objects)};
const index = ${JSON.stringify(index)};
let copied = false;
globalThis.fetch = async (url, options = {}) => {
  const key = String(url).split("/v2/")[1].split("/").slice(2).join("/");
  const method = options.method || "GET";
  appendFileSync(process.env.CALLS, method + " " + key + "\\n");
  const scenario = process.env.SCENARIO;
  const response = (body, status = 200, headers = {}) => new Response(body, { status, headers });
  const missing = () => response(JSON.stringify({ errors: [{ code: "MANIFEST_UNKNOWN" }] }), 404);
  if (method === "PUT") {
    if (scenario === "copy-failure") return response("failed", 500);
    copied = true; return response("", 201);
  }
  if (key === "manifests/" + process.env.REVISION) {
    if (["missing-preview", "preview-missing"].includes(scenario)) return missing();
    if (scenario === "transport-unavailable") throw Error("unavailable");
    if (["bad-digest", "malformed"].includes(scenario)) return response(index, 200, { "docker-content-digest": "sha256:bad" });
  }
  if (key.startsWith("manifests/v")) {
    if (copied && scenario === "changed-after-copy") return response(index + " ", 200);
    if (["existing-different", "conflict"].includes(scenario)) return response(index + " ", 200);
    if (["auth", "unauthorized"].includes(scenario)) return response("unauthorized", 403);
    if (["network", "timeout"].includes(scenario)) throw Error("timeout");
    if (["generic-404", "notfound"].includes(scenario)) return response("404 Not Found", 404);
    if (scenario === "nameunknown") return response(JSON.stringify({ errors: [{ code: "NAME_UNKNOWN" }] }), 404);
    if (copied || ["existing-same", "same"].includes(scenario)) return response(index);
    return missing();
  }
  return objects[key] === undefined ? missing() : response(objects[key]);
};
`;
    writeFileSync(scriptPath, harness + sharedPromotion.replace(/^#!.*\n/, ""));
    writeFileSync(calls, "");
    const result = Bun.spawnSync([process.execPath, scriptPath], {
      env: { ...process.env, DOCKER_CONFIG: directory, REVISION: revision, VERSION: overrides.RELEASE_TAG ?? "v0.1.0", IMAGES: JSON.stringify([{ key: "image", repository, platforms: ["amd64", "arm64"], user: "65532:65532" }]), IMAGES_RESULT: output, SCENARIO: scenario, CALLS: calls },
    });
    const value = existsSync(output) ? JSON.parse(readFileSync(output, "utf8")).image : undefined;
    return { status: result.exitCode, error: result.stderr.toString(), calls: readFileSync(calls, "utf8"), image: value, result: value ?? "" };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

describe("release declaration", () => {
  test("delegates metadata verification and promotion to the installed shared Task", () => {
    const promotion = task("promote-release");
    expect((promotion as any).taskRef).toEqual({ name: "service-promote-v1" });
    expect(promotion.taskSpec).toBeUndefined();
    expect(promotion.params).toEqual([
      { name: "revision", value: "$(params.revision)" },
      { name: "version", value: "$(tasks.validate-release.results.tag)" },
      { name: "images", value: JSON.stringify([{ key: "image", repository: "cr.holdenitdown.net/rfhold/smarthome-mcp", platforms: ["amd64", "arm64"], user: "65532:65532" }]) },
    ]);
  });
  test("all inline shell scripts parse", () => {
    for (const item of tasks) for (const step of (item.taskSpec?.steps ?? [])) {
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
    expect((deploy as any).taskRef).toEqual({ name: "service-pulumi-up-v1" });
    expect(deploy.taskSpec).toBeUndefined();
    expect(Object.fromEntries(deploy.params!.map((param: any) => [param.name, param.value]))).toEqual({
      "source-subdir": "infra/pulumi", stack: "prod", "image-config": "$(tasks.promote-release.results.images)",
      "authentik-enabled": "true", "grafana-enabled": "false", "openbao-enabled": "false",
    });
    expect(yaml).not.toMatch(/pulumi preview|openbao-pulumi|openbao-login|VAULT_TOKEN|serviceAccountName:/);
  });

  test("confirmed prod SSH settings preserve exact auth and storage policy", () => {
    const prod = Bun.YAML.parse(readFileSync(join(root, "infra/pulumi/Pulumi.prod.yaml"), "utf8")) as { config: Record<string, unknown> };
    const expected = {
      homeAssistantSshHost: "172.16.1.10", homeAssistantSshPort: "2200",
      homeAssistantSshUsername: "root", homeAssistantSshConfigRoot: "/homeassistant",
      homeAssistantSshEgressCidr: "172.16.1.10/32",
      protectData: "true", databaseStorageSize: "20Gi", backupRetention: "30d",
    };
    for (const [key, value] of Object.entries(expected)) expect(prod.config[`smarthome-mcp:${key}`]).toBe(value);
    expect(prod.config["smarthome-mcp:mcpOAuthCimdTrustedPrivateOrigins"]).toEqual([
      "https://waltr.holdenitdown.net",
    ]);
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


describe("shared promotion regression with local registry stub", () => {
  for (const scenario of ["absent", "same"]) test(scenario, () => {
    const result = promote(scenario);
    expect(result.status, result.error).toBe(0);
    expect(result.result).toBe(image);
    expect(result.calls).toContain(`GET manifests/${descriptor("absent", "amd64").digest}`);
    expect(result.calls).toContain(`GET manifests/${descriptor("absent", "arm64").digest}`);
    if (scenario === "absent") expect(result.calls).toContain(`PUT manifests/v0.1.0`);
    else expect(result.calls).not.toContain("PUT ");
  });
  for (const scenario of ["transport-unavailable", "preview-missing", "malformed", "single", "missing-arm64", "root", "wrong-revision", "wrong-os", "wrong-architecture", "nested-root", "nested-wrong-revision", "nested-wrong-os", "nested-wrong-architecture", "malformed-config", "multiple-config", "nested-platform", "wrong-platform-os", "manifest-not-array", "malformed-manifest", "multiple-manifest", "conflict", "unauthorized", "timeout", "notfound", "nameunknown"]) test(scenario, () => {
    const result = promote(scenario);
    expect(result.status).not.toBe(0);
    expect(result.calls).not.toContain("PUT ");
    expect(result.result).toBe("");
  });
});
