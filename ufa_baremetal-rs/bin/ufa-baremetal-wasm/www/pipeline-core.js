"use strict";

globalThis.PipelineCore = (() => {
  const kCodes = Object.freeze(["K01", "K02", "K03", "K04", "K05", "K06"]);
  const steps = {
    convert: {
      label: "C → UAGL",
      inputs: {c: {label: "C source", types: ["c"]}},
      outputs: {uagl: "uagl", validation: "text", gaps: "json"},
    },
    validate: {
      label: "Assembler source check",
      inputs: {uagl: {label: "UAGL source", types: ["uagl", "uaglbr"]}},
      outputs: {diagnostics: "json"},
    },
    assemble: {
      label: "UAGL → AArch64",
      inputs: {uagl: {label: "UAGL source", types: ["uagl", "uaglbr"]}},
      outputs: {assembly: "asm"},
    },
    check: {
      label: "K checks",
      inputs: {assembly: {label: "AArch64 assembly", types: ["asm"]}},
      outputs: {verdicts: "json"},
    },
    reverse: {
      label: "UAGL → C",
      inputs: {uagl: {label: "UAGL text", types: ["uagl"]}},
      outputs: {c: "c"},
    },
    fidelity: {
      label: "Fidelity",
      inputs: {
        c: {label: "Original C source", types: ["c"]},
        uagl: {label: "UAGL text", types: ["uagl"]},
      },
      outputs: {report: "text"},
    },
    verify: {
      label: "Verify",
      inputs: {
        assembly: {label: "AArch64 assembly", types: ["asm"]},
        reference: {label: "Reference assembly", types: ["asm"]},
      },
      outputs: {diff: "text"},
    },
  };

  for (const schema of Object.values(steps)) {
    for (const input of Object.values(schema.inputs)) {
      Object.freeze(input.types);
      Object.freeze(input);
    }
    Object.freeze(schema.inputs);
    Object.freeze(schema.outputs);
    Object.freeze(schema);
  }
  Object.freeze(steps);

  const object = value => value !== null && typeof value === "object" && !Array.isArray(value);
  const nonempty = value => typeof value === "string" && value.trim().length > 0;
  const own = (record, key) => object(record) && Object.hasOwn(record, key) ? record[key] : undefined;
  const fromOpened = id => ({source: "opened", id});
  const fromStep = (stepId, output) => ({source: "step", stepId, output});
  const defaultSlots = {c: "c", uagl: "uagl", assembly: "asm", reference: "reference"};
  const artifactTypes = {c: "C (.c)", uagl: "UAGL (.uagl)", uaglbr: "UAGLBR (.uaglbr)", asm: "assembly (.s)"};

  function newStep(type, id) {
    const schema = own(steps, type);
    if (!schema) throw new Error(`Unknown step type: ${type}`);
    if (!nonempty(id)) throw new Error("A step needs a nonempty ID.");
    return {
      id,
      type,
      inputs: Object.fromEntries(Object.keys(schema.inputs).map(key => [key, fromOpened(defaultSlots[key])])),
      options: type === "convert" ? {validate: true} : type === "check" ? {kCodes: [...kCodes]} : {},
    };
  }

  function makePreset(name, {sourceType = "uagl", verify = false, includeFidelity = true, includeReverse = true} = {}) {
    const config = {version: 1, errorPolicy: "continue", steps: []};
    const append = (type, inputs = {}) => {
      const step = newStep(type, type);
      Object.assign(step.inputs, inputs);
      config.steps.push(step);
      return step;
    };
    const convert = () => {
      append("convert");
      return fromStep("convert", "uagl");
    };
    const openedUagl = fromOpened(name === "uaglbr" || sourceType === "uaglbr" ? "uaglbr" : "uagl");
    const generatedAsm = fromStep("assemble", "assembly");
    if (name === "custom") return config;
    if (name === "fidelity") {
      const uagl = convert();
      if (includeFidelity) append("fidelity", {uagl});
      if (includeReverse) append("reverse", {uagl});
      return config;
    }
    if (name === "check-first") {
      append("check");
      append("assemble", {uagl: openedUagl});
      append("verify", {assembly: generatedAsm});
      return config;
    }
    if (!["c", "uagl", "uaglbr"].includes(name)) throw new Error(`Unknown preset: ${name}`);
    const uagl = name === "c" ? convert() : openedUagl;
    append("validate", {uagl});
    append("assemble", {uagl});
    append("check", {assembly: generatedAsm});
    if (verify) append("verify", {assembly: generatedAsm});
    return config;
  }

  function getPriorOutput(previousRun, stepId, output) {
    const result = previousRun?.steps?.find(step => step.id === stepId);
    return own(result?.outputs, output);
  }

  function validArtifact(artifact) {
    return object(artifact) && nonempty(artifact.name) &&
      (artifact.type === "uaglbr" ? artifact.data instanceof Uint8Array : typeof artifact.data === "string");
  }

  function validate(config, opened, {singleStep, previousRun, requireOpened = true} = {}) {
    const errors = [];
    const add = (message, stepId = null, input = null) => errors.push({stepId, input, message});
    if (!object(config) || config.version !== 1 || !Array.isArray(config.steps)) {
      add("Use a pipeline configuration with version 1 and a steps array.");
      return {ok: false, errors};
    }
    if (!["continue", "stop"].includes(config.errorPolicy)) {
      add("Choose a pipeline error policy: continue or stop.");
    }
    if (config.steps.length === 0 && requireOpened) {
      add("Add at least one step to the pipeline.");
    }
    const index = new Map();
    config.steps.forEach((step, position) => {
      if (!object(step) || !nonempty(step.id)) {
        add(`Step ${position + 1} needs a nonempty ID.`);
      } else if (index.has(step.id)) {
        add(`Step ID ${step.id} appears more than once.`, step.id);
      } else {
        index.set(step.id, {step, position});
      }
    });
    if (singleStep !== undefined && !index.has(singleStep)) {
      add(`Step ${singleStep} does not exist.`);
    }
    for (const {step, position} of index.values()) {
      if (singleStep !== undefined && step.id !== singleStep) continue;
      const schema = own(steps, step.type);
      if (!schema) {
        add(`Unknown step type: ${step.type}.`, step.id);
        continue;
      }
      if (!object(step.inputs)) {
        add(`${schema.label} needs input connections.`, step.id);
      }
      if (step.options !== undefined && !object(step.options)) {
        add(`Invalid settings for ${schema.label}.`, step.id);
      }
      const options = object(step.options) ? step.options : {};
      if (step.type === "convert" && options.validate !== undefined && typeof options.validate !== "boolean") {
        add("Converter validation must be on or off.", step.id);
      }
      if (step.type === "check" && options.kCodes !== undefined &&
          (!Array.isArray(options.kCodes) || options.kCodes.length === 0 ||
           options.kCodes.some(code => !kCodes.includes(code)) ||
           new Set(options.kCodes).size !== options.kCodes.length)) {
        add("Select at least one K check. Use each K01–K06 code once.", step.id);
      }
      for (const [key, input] of Object.entries(schema.inputs)) {
        const ref = own(step.inputs, key);
        const expected = input.types.map(type => artifactTypes[type]).join(" or ");
        const problem = message => add(`${input.label}: ${message}`, step.id, key);
        if (ref?.source === "opened" && nonempty(ref.id)) {
          if (!requireOpened) continue;
          const artifact = own(opened, ref.id);
          if (!validArtifact(artifact)) {
            problem(`Open a ${expected} file in the “${ref.id}” slot, or connect an earlier output.`);
          } else if (!input.types.includes(artifact.type)) {
            problem(`${artifact.name} does not match the required type ${expected}.`);
          }
        } else if (ref?.source === "step" && nonempty(ref.stepId) && nonempty(ref.output)) {
          const producer = index.get(ref.stepId);
          const outputs = own(steps, producer?.step.type)?.outputs;
          const outputType = own(outputs, ref.output);
          if (!producer || producer.position >= position) {
            problem(`Connect an output of a step before ${schema.label}; “${ref.stepId}” is unavailable here.`);
          } else if (!outputType) {
            problem(`Step ${ref.stepId} has no “${ref.output}” output.`);
          } else if (!input.types.includes(outputType)) {
            problem(`Output ${ref.stepId}.${ref.output} does not match the required type ${expected}.`);
          } else if (requireOpened && singleStep !== undefined) {
            const artifact = getPriorOutput(previousRun, ref.stepId, ref.output);
            if (!validArtifact(artifact) || !input.types.includes(artifact.type)) {
              problem(`Run ${ref.stepId} to produce “${ref.output}”, or connect an opened file.`);
            }
          }
        } else {
          problem(`Connect an opened ${expected} file or a compatible output from an earlier step.`);
        }
      }
    }
    return {ok: errors.length === 0, errors};
  }

  function copyStep(step) {
    const schema = steps[step.type];
    const inputs = Object.fromEntries(Object.keys(schema.inputs).map(key => {
      const ref = step.inputs[key];
      return [key, ref.source === "opened" ? fromOpened(ref.id) : fromStep(ref.stepId, ref.output)];
    }));
    const options = {};
    if (step.type === "convert") options.validate = step.options?.validate ?? true;
    if (step.type === "check") options.kCodes = [...(step.options?.kCodes ?? kCodes)];
    return {id: step.id, type: step.type, inputs, options};
  }

  function configOnly(config) {
    const validation = validate(config, undefined, {requireOpened: false});
    if (!validation.ok) throw new Error(validation.errors.map(error => error.message).join("\n"));
    return {
      version: 1,
      errorPolicy: config.errorPolicy,
      steps: config.steps.map(copyStep),
    };
  }

  function serializeConfig(config) {
    return JSON.stringify(configOnly(config), null, 2);
  }

  function parseConfig(text) {
    return configOnly(JSON.parse(text));
  }

  function lineDiff(actualText, referenceText) {
    const normalize = text => text.replace(/\r\n?/g, "\n");
    const actual = normalize(actualText);
    const reference = normalize(referenceText);
    if (actual === reference) return null;
    const expectedLines = reference.split("\n");
    const actualLines = actual.split("\n");
    let start = 0;
    let oldEnd = expectedLines.length;
    let newEnd = actualLines.length;
    while (start < oldEnd && start < newEnd && expectedLines[start] === actualLines[start]) start++;
    while (oldEnd > start && newEnd > start && expectedLines[oldEnd - 1] === actualLines[newEnd - 1]) {
      oldEnd--;
      newEnd--;
    }

    const oldLines = expectedLines.slice(start, oldEnd);
    const newLines = actualLines.slice(start, newEnd);
    const limit = 24;
    const output = [];
    const format = (sign, index, line) => `${sign} ${start + index + 1}: ${line || "(empty line)"}`;
    const width = newLines.length + 1;
    const cells = (oldLines.length + 1) * width;
    if (cells > 300000) {
      output.push(`Large differing region beginning at line ${start + 1}. First lines of each region:`);
      output.push(...oldLines.slice(0, 12).map((line, index) => format("-", index, line)));
      output.push(...newLines.slice(0, 12).map((line, index) => format("+", index, line)));
      return output.join("\n");
    }

    const lcs = new Uint32Array(cells);
    for (let a = oldLines.length - 1; a >= 0; a--) {
      for (let b = newLines.length - 1; b >= 0; b--) {
        lcs[a * width + b] = oldLines[a] === newLines[b] ? 1 + lcs[(a + 1) * width + b + 1]
          : Math.max(lcs[(a + 1) * width + b], lcs[a * width + b + 1]);
      }
    }
    let a = 0;
    let b = 0;
    while ((a < oldLines.length || b < newLines.length) && output.length < limit) {
      if (a < oldLines.length && b < newLines.length && oldLines[a] === newLines[b]) {
        a++;
        b++;
      } else if (a < oldLines.length && (b === newLines.length || lcs[(a + 1) * width + b] >= lcs[a * width + b + 1])) {
        output.push(format("-", a, oldLines[a]));
        a++;
      } else {
        output.push(format("+", b, newLines[b]));
        b++;
      }
    }
    if (a < oldLines.length || b < newLines.length) output.push("More differences follow.");
    return output.join("\n");
  }

  const output = (type, name, data) => ({type, name, data});
  const baseName = name => name.replace(/(?:\.uagl)?\.uaglbr$|\.(?:uagl|c|s)$/i, "");
  function cStem(name) {
    const filename = name.split("/").filter(Boolean).at(-1) ?? name;
    const extension = filename.lastIndexOf(".");
    return extension > 0 ? filename.slice(0, extension) : filename;
  }
  const lineNumber = value => Number.isInteger(value) && value > 0 ? value : undefined;

  function assemblerKind(message) {
    if (message.startsWith("Unrecognized UAGL line:")) return "UNRECOGNIZED_UAGL_LINE";
    if (message.startsWith("Unresolved expression:")) return "UNRESOLVED_EXPRESSION";
    if (/^__(?:memcpy|memset|asm__)\s/.test(message)) return "BUILTIN_ARGUMENTS";
    return "SOURCE_DIAGNOSTIC";
  }

  function converterValidation(text, name) {
    const findings = [];
    for (const line of text.split("\n")) {
      const match = /^\s*(ERROR|WARN)\s+L(\d+) \[([^\]]+)\]:\s*(.*)$/.exec(line);
      if (!match) continue;
      const [, level, number, kind, message] = match;
      findings.push({
        source: "converter-validation",
        kind,
        severity: level === "ERROR" ? "error" : "warning",
        message,
        uaglFile: name,
        uaglLine: lineNumber(Number(number)),
      });
    }
    if (text.startsWith("❌ FAIL") && !findings.some(finding => finding.severity === "error")) {
      findings.push({source: "converter-validation", kind: "VALIDATION_FAILED", severity: "error", message: text});
    }
    return findings;
  }

  function converterGaps(text, cFile, uaglFile) {
    const manifest = JSON.parse(text);
    if (!Array.isArray(manifest?.gaps)) throw new Error("Invalid converter gap manifest.");
    return manifest.gaps.map(gap => {
      const message = [gap.note, gap.c].filter(Boolean).join(" · ");
      return {
        source: "converter-gap",
        kind: gap.kind,
        severity: gap.severity,
        message,
        function: gap.function || undefined,
        cFile,
        uaglFile,
        cLine: lineNumber(gap.c_line ?? gap.cLine ?? gap.c_location?.line),
        uaglLine: lineNumber(gap.uagl_line ?? gap.uaglLine ?? gap.uagl_location?.line),
        raw: gap,
      };
    });
  }

  function selectInputs(step, opened, previousRun) {
    const inputs = {};
    const missing = [];
    for (const [key, schema] of Object.entries(steps[step.type].inputs)) {
      const ref = step.inputs[key];
      const value = ref.source === "opened" ? own(opened, ref.id)
        : getPriorOutput(previousRun, ref.stepId, ref.output);
      if (validArtifact(value) && schema.types.includes(value.type)) {
        inputs[key] = value;
      } else {
        const location = ref.source === "opened" ? ref.id : `${ref.stepId}.${ref.output}`;
        missing.push(`${schema.label} is unavailable (${location}).`);
      }
    }
    return {inputs, missing};
  }

  function blankStep(step, inputs = {}) {
    return {id: step.id, type: step.type, status: "pass", message: "", inputs, outputs: {}, diagnostics: []};
  }

  async function execute(step, opened, previousRun, tools, stoppedAt) {
    const {inputs, missing} = selectInputs(step, opened, previousRun);
    const result = blankStep(step, inputs);
    const {c, uagl, assembly, reference} = inputs;
    const add = diagnostic => result.diagnostics.push({...diagnostic, stepId: step.id});
    if (stoppedAt || missing.length) {
      result.status = "skipped";
      result.message = stoppedAt ? `Stopped after ${stoppedAt} failed.` : missing.join(" ");
      return result;
    }
    try {
      switch (step.type) {
        case "convert": {
          const payload = await tools.convert(c.data, c.name, step.options?.validate ?? true);
          if (typeof payload?.uagl !== "string") throw new Error("Invalid C converter response.");
          const basename = cStem(c.name);
          const uaglName = `${basename}.uagl`;
          result.outputs.uagl = output("uagl", uaglName, payload.uagl);
          if (typeof payload.gaps_json !== "string") throw new Error("The C converter did not return a gap manifest.");
          result.outputs.gaps = output("json", `${basename}.gaps.json`, payload.gaps_json);
          converterGaps(payload.gaps_json, c.name, uaglName).forEach(add);
          if (step.options.validate) {
            if (typeof payload.validation !== "string") throw new Error("The C converter did not return a validation report.");
            result.outputs.validation = output("text", `${basename}.validation.txt`, payload.validation);
            converterValidation(payload.validation, uaglName).forEach(add);
          }
          result.message = `Converted ${c.name} to ${uaglName}.`;
          break;
        }
        case "validate": {
          const payload = await tools.validate(uagl.data, uagl.name);
          if (!Array.isArray(payload?.diagnostics)) throw new Error("Invalid assembler source-check response.");
          result.outputs.diagnostics = output("json", `${baseName(uagl.name)}.assembler-diagnostics.json`,
            JSON.stringify(payload.diagnostics, null, 2));
          for (const finding of payload.diagnostics) {
            add({
              source: "assembler-validation",
              kind: finding.kind ?? assemblerKind(finding.message),
              severity: "error",
              message: finding.message,
              function: finding.function || undefined,
              uaglFile: uagl.name,
              uaglLine: lineNumber(finding.line),
              cLine: lineNumber(finding.c_line),
              raw: finding,
            });
          }
          result.message = payload.diagnostics.length === 0 ? `${uagl.name} has no assembler source diagnostics.`
            : `${uagl.name} has ${payload.diagnostics.length} assembler source diagnostic(s).`;
          break;
        }
        case "assemble": {
          const payload = await tools.assemble(uagl.data, uagl.name);
          if (typeof payload?.assembly !== "string") throw new Error("Invalid assembler response.");
          const name = `${baseName(uagl.name)}.s`;
          result.outputs.assembly = output("asm", name, payload.assembly);
          result.message = `Generated ${name}.`;
          break;
        }
        case "check": {
          const selection = step.options.kCodes;
          const mask = selection.reduce((mask, code) => mask | (1 << kCodes.indexOf(code)), 0);
          const verdicts = await tools.check(assembly.data, assembly.name, mask);
          if (!Array.isArray(verdicts)) throw new Error("Invalid K-check response.");
          result.outputs.verdicts = output("json", `${baseName(assembly.name)}.k-checks.json`, JSON.stringify(verdicts, null, 2));
          for (const verdict of verdicts) {
            add({
              source: "k-check",
              kind: verdict.k_class,
              severity: verdict.status === "FAIL" ? "error" : "info",
              message: verdict.diagnostic,
              function: verdict.function || undefined,
              asmFile: assembly.name,
              asmLine: lineNumber(verdict.line_no),
              kCode: verdict.k_class,
              verdictStatus: verdict.status,
              suggestedFix: verdict.suggested_fix || undefined,
              raw: verdict,
            });
          }
          result.message = `Checked ${assembly.name} with ${selection.join(", ")}.`;
          break;
        }
        case "reverse": {
          const payload = await tools.reverse(uagl.data);
          if (typeof payload?.c !== "string") throw new Error("Invalid reverse converter response.");
          const name = `${baseName(uagl.name)}.roundtrip.c`;
          result.outputs.c = output("c", name, payload.c);
          result.message = `Generated ${name}.`;
          break;
        }
        case "fidelity": {
          const payload = await tools.fidelity(c.data, uagl.data, c.name, uagl.name);
          if (typeof payload?.report !== "string") throw new Error("Invalid fidelity response.");
          const name = `${baseName(uagl.name)}.fidelity.md`;
          result.outputs.report = output("text", name, payload.report);
          result.message = `Compared ${c.name} and ${uagl.name}.`;
          break;
        }
        case "verify": {
          const diff = lineDiff(assembly.data, reference.data);
          result.message = diff === null ? `${assembly.name} matches ${reference.name}.`
            : `${assembly.name} differs from ${reference.name}. − reference, + pipeline output.`;
          result.outputs.diff = output("text", `${baseName(assembly.name)}.verify.txt`, diff ?? result.message);
          if (diff !== null) {
            add({source: "verify", kind: "ASSEMBLY_DIFFERENCE", severity: "error", message: result.message});
          }
          break;
        }
      }
    } catch (error) {
      result.message = error instanceof Error ? error.message : String(error);
      add({source: "pipeline", kind: "STEP_ERROR", severity: "error", message: result.message});
    }
    if (result.diagnostics.some(finding => finding.severity === "error")) {
      result.status = "fail";
    } else if (result.diagnostics.some(finding => ["warning", "review", "needs-ai"].includes(finding.severity))) {
      result.status = "warn";
    }
    return result;
  }

  function runResult(results, errors = [], stoppedAt = null) {
    const status = errors.length || results.some(step => step.status === "fail") ? "fail"
      : results.some(step => step.status === "warn") ? "warn"
        : results.every(step => step.status === "skipped") ? "skipped" : "pass";
    const audit = results.flatMap(result => result.diagnostics).filter(diagnostic =>
      ["converter-gap", "converter-validation", "assembler-validation"].includes(diagnostic.source));
    return {status, steps: results, audit, errors, stoppedAt};
  }

  async function run(config, opened, tools, {onStep} = {}) {
    const validation = validate(config, opened);
    if (!validation.ok) return runResult([], validation.errors);
    const configuredSteps = config.steps.map(copyStep);
    const {errorPolicy} = config;
    const openedSnapshot = {...opened};
    const results = [];
    let stoppedAt = null;
    for (const step of configuredSteps) {
      const result = await execute(step, openedSnapshot, {steps: results}, tools, stoppedAt);
      results.push(result);
      onStep?.(result);
      if (errorPolicy === "stop" && result.status === "fail") stoppedAt = step.id;
    }
    return runResult(results, [], stoppedAt);
  }

  async function runStep(config, id, opened, tools, previousRun, {onStep} = {}) {
    const validation = validate(config, opened, {singleStep: id, previousRun});
    if (!validation.ok) return runResult([], validation.errors);
    const step = copyStep(config.steps.find(step => step.id === id));
    const result = await execute(step, opened, previousRun, tools);
    onStep?.(result);
    return runResult([result]);
  }

  return Object.freeze({steps, kCodes, makePreset, newStep, validate, serializeConfig, parseConfig, run, runStep, lineDiff});
})();
