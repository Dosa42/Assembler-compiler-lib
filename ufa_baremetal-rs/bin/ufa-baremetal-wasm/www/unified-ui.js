(() => {
  "use strict";

  const core = globalThis.PipelineCore;
  const tools = globalThis.WasmTools.create({
    converter: CONVERTER_WASM_BASE64,
    assembler: ASSEMBLER_WASM_BASE64,
    checker: WASM_BASE64,
  });
  const byId = id => document.getElementById(id);
  const inputKind = byId("input-kind");
  const inputFile = byId("input-file");
  const inputName = byId("input-name");
  const inputText = byId("input-text");
  const mode = byId("pipeline-mode");
  const verify = byId("pipeline-verify");
  const converterValidation = byId("converter-validation");
  const stopOnError = byId("stop-on-error");
  const pipelineRun = byId("pipeline-run");
  const stepList = byId("pipeline-steps");
  const addStep = byId("pipeline-add-step");
  const manualOperation = byId("manual-operation");
  const checkboxes = [...document.querySelectorAll('input[name="k-check"]')];
  const results = byId("pipeline-results");
  const originals = Object.create(null);
  const draftNames = Object.create(null);
  const reads = new Map();
  const inputSpecs = {
    c: { type: "c", name: "invoer.c", accept: ".c,.h", label: "C-bron" },
    uagl: { type: "uagl", name: "invoer.uagl", accept: ".uagl", label: "UAGL" },
    uaglbr: { type: "uaglbr", name: "invoer.uaglbr", accept: ".uaglbr", label: "UAGLBR" },
    asm: { type: "asm", name: "invoer.s", accept: ".s", label: "Assembly" },
    reference: { type: "asm", name: "referentie.s", accept: ".s", label: "Referentie" },
  };
  const labels = {
    convert: "C → UAGL",
    validate: "UAGL-broncontrole",
    assemble: "UAGL → AArch64",
    check: "K-checks",
    reverse: "UAGL → C",
    fidelity: "Fidelity",
    verify: "Verify",
  };
  const outputLabels = {
    uagl: "UAGL", validation: "Convertervalidatie", gaps: "Convertergaten",
    diagnostics: "Brondiagnoses", assembly: "Assembly", verdicts: "K-resultaten",
    c: "C", report: "Fidelity-rapport", diff: "Vergelijking",
  };
  const inputLabels = {
    c: "C-bron", uagl: "UAGL-bron", assembly: "Assembly", reference: "Referentie-assembly",
  };
  const statusLabels = {
    pass: "Geslaagd", warn: "Met meldingen", fail: "Mislukt", skipped: "Overgeslagen",
    warning: "Waarschuwing", error: "Fout", info: "Info", stopped: "Gestopt",
  };
  const allChecks = () => checkboxes.filter(control => control.checked || control.indeterminate).map(control => control.value);
  const expandedArtifacts = new Set();
  let config;
  let currentMode = mode.value;
  let savedCustom;
  let manualStep;
  let lastPipeline;
  let lastManual;
  let revision = 0;
  let manualRevision = 0;
  let nextStepId = 1;
  let busy = false;
  let inProgress;
  let runMessage = "";

  function make(tag, text, className) {
    const element = document.createElement(tag);
    if (text !== undefined) element.textContent = text;
    if (className) element.className = className;
    return element;
  }

  function button(text, handler, label) {
    const element = make("button", text);
    element.type = "button";
    if (label) element.setAttribute("aria-label", label);
    element.addEventListener("click", handler);
    return element;
  }

  function status(id, message, error = false) {
    const element = byId(id);
    element.textContent = message;
    element.dataset.state = error ? "error" : "";
  }

  function newId() {
    const used = new Set([
      ...config.steps.map(step => step.id),
      ...(lastPipeline?.config.steps.map(step => step.id) ?? []),
      ...(lastManual?.config.steps.map(step => step.id) ?? []),
    ]);
    let id;
    do {
      id = "step-" + nextStepId++;
    } while (used.has(id));
    return id;
  }

  function cloneConfig(value) {
    return JSON.parse(JSON.stringify(value));
  }

  function refKey(ref) {
    if (!ref) return "";
    return ref.source === "opened"
      ? JSON.stringify(["opened", ref.id])
      : JSON.stringify(["step", ref.stepId, ref.output]);
  }

  function selectedName() {
    const spec = inputSpecs[inputKind.value];
    return inputName.value.trim() || spec.name;
  }

  function showInput() {
    const id = inputKind.value;
    const spec = inputSpecs[id];
    const entry = originals[id];
    inputFile.accept = spec.accept;
    inputFile.value = "";
    inputName.value = entry?.name ?? draftNames[id] ?? spec.name;
    inputText.value = spec.type === "uaglbr" ? "" : entry?.data ?? "";
    byId("input-editor").hidden = spec.type === "uaglbr";
    byId("input-clear").disabled = !entry && !reads.has(id);
    byId("input-download").disabled = !entry;
    const message = reads.has(id) ? "Bestand wordt geopend…" : entry
      ? spec.type === "uaglbr" ? entry.name + " · " + entry.data.length + " bytes" : entry.name
      : id === "asm" || id === "reference" ? "Voor K05 telt de bestandsnaam van de assembly." : "";
    status("input-status", message);
  }

  function invalidate({ steps = false, manual = true, scope = "both" } = {}) {
    if (scope !== "manual") {
      revision++;
      runMessage = "";
    }
    if (scope !== "pipeline") manualRevision++;
    if (steps) renderSteps();
    if (manual) renderManualInputs();
    renderPreflight();
    renderResults();
  }

  function commitInput(id, entry, options) {
    if (entry) originals[id] = entry;
    else delete originals[id];
    invalidate({ steps: true, ...options });
  }

  function bindInput() {
    inputKind.addEventListener("change", showInput);
    inputFile.addEventListener("change", async () => {
      const file = inputFile.files?.[0];
      if (!file) return;
      const id = inputKind.value;
      const spec = inputSpecs[id];
      const ticket = Symbol(id);
      reads.set(id, ticket);
      status("input-status", file.name + " wordt geopend…");
      byId("input-clear").disabled = false;
      renderPreflight();
      try {
        const extension = /\.[^.]+$/.exec(file.name)?.[0]?.toLowerCase();
        if (!spec.accept.split(",").includes(extension)) {
          throw new Error("Kies " + spec.accept.replace(",", " of ") + " voor " + spec.label + ".");
        }
        const bytes = new Uint8Array(await file.arrayBuffer());
        if (reads.get(id) !== ticket) return;
        reads.delete(id);
        const data = spec.type === "uaglbr" ? bytes : new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes);
        draftNames[id] = file.name;
        commitInput(id, { type: spec.type, name: file.name, data });
        if (id === inputKind.value) showInput();
      } catch (error) {
        if (reads.get(id) !== ticket) return;
        reads.delete(id);
        if (id === inputKind.value) {
          showInput();
          status("input-status", error.message, true);
        }
        renderPreflight();
      }
    });
    inputText.addEventListener("input", () => {
      const id = inputKind.value;
      const spec = inputSpecs[id];
      reads.delete(id);
      inputFile.value = "";
      const name = selectedName();
      const data = inputText.value;
      draftNames[id] = name;
      commitInput(id, data.length ? { type: spec.type, name, data } : undefined);
      byId("input-clear").disabled = !originals[id];
      byId("input-download").disabled = !originals[id];
      status("input-status", originals[id] ? name : "");
    });
    inputName.addEventListener("input", () => {
      const id = inputKind.value;
      const name = selectedName();
      draftNames[id] = name;
      const original = originals[id];
      if (original && original.name !== name) {
        commitInput(id, { ...original, name });
        status("input-status", original.type === "uaglbr" ? name + " · " + original.data.length + " bytes" : name);
      }
    });
    byId("input-clear").addEventListener("click", () => {
      reads.delete(inputKind.value);
      delete draftNames[inputKind.value];
      commitInput(inputKind.value, undefined);
      showInput();
      inputFile.focus();
    });
    byId("input-download").addEventListener("click", () => {
      const entry = originals[inputKind.value];
      if (entry) download(entry.name, entry.data);
    });
  }

  function compatibleChoices(declaration, priorSteps, availableOutputs) {
    const choices = Object.entries(inputSpecs)
      .filter(([, spec]) => declaration.types.includes(spec.type))
      .map(([id, spec]) => ({
        ref: { source: "opened", id },
        label: originals[id] ? spec.label + " · " + originals[id].name : spec.label + " · open bij Invoer",
      }));
    for (const [index, step] of priorSteps.entries()) {
      const outputTypes = core.steps[step.type]?.outputs ?? {};
      for (const [output, type] of Object.entries(outputTypes)) {
        if (!declaration.types.includes(type)) continue;
        const artifact = availableOutputs?.get(step.id)?.outputs[output];
        if (availableOutputs && !artifact) continue;
        const artifactName = artifact ? " · " + artifact.name : "";
        choices.push({
          ref: { source: "step", stepId: step.id, output },
          label: `Stap ${index + 1} · ${labels[step.type]} · ${outputLabels[output] ?? output}${artifactName}`,
        });
      }
    }
    return choices;
  }

  function slotField(step, slot, declaration, choices, manual = false) {
    const field = make("div", undefined, "field");
    const label = make("label", inputLabels[slot] ?? declaration.label ?? slot);
    const select = make("select", undefined, "slot-select");
    select.id = (manual ? "manual-" : "") + encodeURIComponent(step.id) + "-slot-" + slot;
    select.dataset.slot = slot;
    select.dataset.stepId = step.id;
    label.htmlFor = select.id;
    const empty = make("option", "Kies invoer");
    empty.value = "";
    select.append(empty);
    for (const choice of choices) {
      const option = make("option", choice.label);
      option.value = refKey(choice.ref);
      select.append(option);
    }
    const bound = refKey(step.inputs[slot]);
    if (bound && !choices.some(choice => refKey(choice.ref) === bound)) {
      const missing = make("option", "Gekoppelde invoer is hier niet beschikbaar");
      missing.value = bound;
      select.append(missing);
    }
    select.value = bound;
    select.addEventListener("change", () => {
      const choice = choices.find(item => refKey(item.ref) === select.value);
      if (choice) step.inputs[slot] = choice.ref;
      else delete step.inputs[slot];
      invalidate({ manual: false, scope: manual ? "manual" : "pipeline" });
    });
    field.append(label, select);
    return field;
  }

  function makePreset(name) {
    const settings = {
      sourceType: name === "uaglbr" ? "uaglbr" : "uagl",
      verify: verify.checked,
      includeFidelity: true,
      includeReverse: true,
    };
    const preset = core.makePreset(name, settings);
    preset.errorPolicy = stopOnError.checked ? "stop" : "continue";
    preset.steps.forEach(applyDefaultOptions);
    return preset;
  }

  function applyDefaultOptions(step) {
    step.options ??= {};
    if (step.type === "convert") step.options.validate = converterValidation.checked || converterValidation.indeterminate;
    if (step.type === "check") step.options.kCodes = allChecks();
  }

  function updateKCode(step, code, enabled) {
    const codes = new Set(step.options.kCodes ?? core.kCodes);
    if (enabled) codes.add(code);
    else codes.delete(code);
    step.options.kCodes = core.kCodes.filter(item => codes.has(item));
  }

  function stepOptions(step) {
    const fieldset = make("fieldset");
    const legend = make("legend", step.type === "check" ? "K-selectie voor deze stap" : "Instellingen voor deze stap");
    const controls = make("div", undefined, "row");
    const options = step.type === "check" ? core.kCodes : ["Convertervalidatie"];
    for (const item of options) {
      const label = make("label", undefined, "choice");
      const input = make("input");
      input.type = "checkbox";
      input.dataset.stepOption = item;
      input.checked = step.type === "check" ? (step.options.kCodes ?? core.kCodes).includes(item) : step.options.validate !== false;
      input.addEventListener("change", () => {
        if (step.type === "check") updateKCode(step, item, input.checked);
        else step.options.validate = input.checked;
        applyControls();
        invalidate({ manual: false, scope: "pipeline" });
      });
      label.append(input, document.createTextNode(item));
      controls.append(label);
    }
    fieldset.append(legend, controls);
    return fieldset;
  }

  function renderSteps() {
    const isCustom = mode.value === "custom";
    byId("pipeline-custom").hidden = !isCustom;
    byId("pipeline-verify-option").hidden = !["c", "uagl", "uaglbr"].includes(mode.value);
    stepList.replaceChildren();
    for (const [index, step] of config.steps.entries()) {
      const row = make("li", undefined, "step-row");
      row.dataset.stepId = step.id;
      row.dataset.stepType = step.type;
      const heading = make("div", undefined, "row between step-title");
      heading.append(make("h3", labels[step.type]));
      if (isCustom) {
        const controls = make("div", undefined, "row");
        for (const [action, text, delta] of [["up", "Omhoog", -1], ["down", "Omlaag", 1], ["remove", "Verwijder", 0]]) {
          const control = button(text, () => {
            if (action === "remove") config.steps.splice(index, 1);
            else [config.steps[index], config.steps[index + delta]] = [config.steps[index + delta], config.steps[index]];
            invalidate({ steps: true, scope: "pipeline" });
            const targetId = action === "remove" ? config.steps[index]?.id ?? config.steps[index - 1]?.id : step.id;
            const newRow = [...stepList.children].find(child => child.dataset.stepId === targetId);
            (newRow?.querySelector("button[data-action='" + action + "']:not(:disabled)") ??
              newRow?.querySelector("button:not(:disabled)") ?? addStep).focus();
          }, text + " · stap " + (index + 1) + " " + labels[step.type]);
          control.dataset.action = action;
          control.disabled = action === "up" && index === 0 || action === "down" && index === config.steps.length - 1;
          controls.append(control);
        }
        heading.append(controls);
      }
      row.append(heading);
      for (const [slot, declaration] of Object.entries(core.steps[step.type].inputs)) {
        const choices = compatibleChoices(declaration, config.steps.slice(0, index));
        row.append(slotField(step, slot, declaration, choices));
      }
      if (isCustom && ["check", "convert"].includes(step.type)) row.append(stepOptions(step));
      stepList.append(row);
    }
  }

  function manualContext() {
    const snapshots = [lastPipeline, lastManual].filter(Boolean);
    const producerSteps = new Map();
    const producerResults = new Map();
    for (const snapshot of snapshots) {
      const currentIds = new Set(snapshot.run.steps.filter(step => step.id !== manualStep?.id).map(step => step.id));
      for (const step of snapshot.config.steps) {
        if (currentIds.has(step.id)) producerSteps.set(step.id, step);
      }
      for (const step of snapshot.run.steps) {
        if (currentIds.has(step.id)) producerResults.set(step.id, step);
      }
    }
    return {
      steps: [...producerSteps.values()],
      outputs: producerResults,
      previousRun: { steps: [...producerResults.values()] },
    };
  }

  function renderManualInputs() {
    const context = manualContext();
    const container = byId("manual-inputs");
    container.replaceChildren();
    for (const [slot, declaration] of Object.entries(core.steps[manualStep.type].inputs)) {
      const choices = compatibleChoices(declaration, context.steps, context.outputs);
      container.append(slotField(manualStep, slot, declaration, choices, true));
    }
  }

  function manualConfig() {
    const context = manualContext();
    return {
      config: {
        version: 1,
        errorPolicy: config.errorPolicy,
        steps: [...context.steps, manualStep],
      },
      previousRun: context.previousRun,
    };
  }

  function describeError(problem, steps) {
    const step = steps.find(item => item.id === problem.stepId);
    const index = steps.indexOf(step);
    const name = index >= 0 ? "Stap " + (index + 1) + " · " + labels[step.type] + " · " : "";
    return name + (inputLabels[problem.input] ? inputLabels[problem.input] + " · " : "") + problem.message;
  }

  function preflight() {
    try {
      return core.validate(config, originals);
    } catch (error) {
      return { ok: false, errors: [{ message: error.message }] };
    }
  }

  function singlePreflight() {
    const manual = manualConfig();
    try {
      return core.validate(manual.config, originals, {
        singleStep: manualStep.id,
        previousRun: manual.previousRun,
      });
    } catch (error) {
      return { ok: false, errors: [{ message: error.message }] };
    }
  }

  function renderPreflight() {
    pipelineRun.disabled = busy || reads.size > 0;
    byId("manual-run").disabled = busy || reads.size > 0;
    if (busy || reads.size) {
      status("pipeline-status", busy ? "Bezig…" : "Wacht tot het bestand geopend is.");
      return;
    }
    const check = preflight();
    const single = singlePreflight();
    if (!runMessage) {
      status("pipeline-status", check.ok ? "Klaar om uit te voeren." :
        check.errors.map(error => describeError(error, config.steps)).join(" "), !check.ok);
    }
    status("manual-status", single.ok ? "Klaar om uit te voeren." : single.errors.map(error => error.message).join(" "), !single.ok);
  }

  function focusProblem(problem) {
    if (!problem) return;
    const row = [...stepList.children].find(item => item.dataset.stepId === problem.stepId);
    const select = [...(row?.querySelectorAll("select") ?? [])].find(item => item.dataset.slot === problem.input);
    (select ?? stepList.querySelector("select") ?? mode).focus();
  }

  function summarizeRun(run) {
    const failed = run.steps.filter(step => step.status === "fail").length;
    const skipped = run.steps.filter(step => step.status === "skipped").length;
    return run.steps.length + " stap(pen) · " + failed + " mislukt · " + skipped + " overgeslagen";
  }

  async function runPipeline() {
    if (busy || reads.size) return;
    const check = preflight();
    if (!check.ok) {
      focusProblem(check.errors[0]);
      return;
    }
    const snapshot = cloneConfig(config);
    const opened = { ...originals };
    const record = { config: snapshot, revision, run: { steps: [], audit: [] } };
    busy = true;
    inProgress = record;
    renderPreflight();
    renderResults();
    await new Promise(resolve => setTimeout(resolve, 0));
    try {
      record.run = await core.run(snapshot, opened, tools, {
        onStep(step) {
          record.run.steps.push(step);
          renderResults();
        },
      });
      lastPipeline = record;
      runMessage = summarizeRun(record.run);
      status("pipeline-status", runMessage, record.run.steps.some(step => step.status === "fail"));
    } catch (error) {
      runMessage = error.message;
      status("pipeline-status", runMessage, true);
    } finally {
      inProgress = undefined;
      busy = false;
      renderManualInputs();
      renderPreflight();
      renderResults();
    }
  }

  async function runManual() {
    if (busy || reads.size) return;
    const check = singlePreflight();
    if (!check.ok) {
      byId("manual-inputs").querySelector("select")?.focus();
      return;
    }
    const manual = manualConfig();
    const snapshot = cloneConfig(manual.config);
    const id = manualStep.id;
    const opened = { ...originals };
    const record = { config: snapshot, revision: manualRevision, manual: true, run: { steps: [], audit: [] } };
    busy = true;
    inProgress = record;
    renderPreflight();
    renderResults();
    await new Promise(resolve => setTimeout(resolve, 0));
    let message;
    let error = false;
    try {
      record.run = await core.runStep(snapshot, id, opened, tools, manual.previousRun);
      lastManual = record;
      message = summarizeRun(record.run);
      error = record.run.steps.some(step => step.status === "fail");
    } catch (failure) {
      error = true;
      message = failure.message;
    } finally {
      inProgress = undefined;
      busy = false;
      renderManualInputs();
      renderPreflight();
      renderResults();
      status("manual-status", message, error);
    }
  }

  function fileText(data) {
    if (typeof data === "string") return data;
    return JSON.stringify(data, null, 2);
  }

  function download(name, data, type = "text/plain;charset=utf-8") {
    const content = data instanceof Uint8Array ? data : fileText(data);
    const url = URL.createObjectURL(new Blob([content], { type }));
    const link = make("a");
    link.href = url;
    link.download = name.split(/[\\/]/).at(-1);
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  function diagLocation(diagnostic) {
    return [
      diagnostic.function,
      diagnostic.kCode,
      diagnostic.cLine ? "C-regel " + diagnostic.cLine : "",
      diagnostic.uaglLine ? "UAGL-regel " + diagnostic.uaglLine : "",
      diagnostic.asmLine ? "Assemblyregel " + diagnostic.asmLine : "",
    ].filter(Boolean).join(" · ");
  }

  function diagnosticVisible(diagnostic) {
    const filter = byId("diagnostic-filter").value.trim().toLowerCase();
    const k = byId("k-filter").value;
    return (!filter || (diagnostic.function ?? "").toLowerCase().includes(filter))
      && (!k || diagnostic.kCode === k);
  }

  function badge(text, state) {
    const label = make("span", text, "badge");
    label.dataset.status = state;
    return label;
  }

  function renderDiagnostics(list, diagnostics, audit = false) {
    list.replaceChildren();
    for (const item of diagnostics.filter(diagnosticVisible)) {
      const line = make("li");
      const category = [item.verdictStatus ?? statusLabels[item.severity] ?? item.severity, item.kind].filter(Boolean);
      const origin = (audit ? [item.source, ...category] : category).filter(Boolean).join(" · ");
      const location = diagLocation(item);
      if (origin) line.append(badge(origin, item.severity));
      line.append(document.createTextNode((location ? location + " · " : "") + item.message));
      if (item.suggestedFix) line.append(make("p", item.suggestedFix, "subtle"));
      list.append(line);
    }
    if (diagnostics.length && !list.childElementCount) list.append(make("li", "Geen meldingen voor dit filter."));
  }

  function addArtifact(row, key, artifact, previewKey) {
    const group = make("div", undefined, "artifact stack");
    const toolbar = make("div", undefined, "row between");
    toolbar.append(make("span", (outputLabels[key] ?? key) + " · " + artifact.name, "artifact-name"));
    toolbar.append(button("Download", () => download(artifact.name, artifact.data,
      artifact.type === "json" ? "application/json" : "text/plain;charset=utf-8"), "Download " + artifact.name));
    const preview = make("details");
    preview.append(make("summary", "Bekijk uitvoer"));
    function loadPreview() {
      if (preview.querySelector("pre")) return;
      const pre = make("pre", fileText(artifact.data), "preview");
      pre.tabIndex = 0;
      preview.append(pre);
    }
    if (expandedArtifacts.has(previewKey)) {
      preview.open = true;
      loadPreview();
    }
    preview.addEventListener("toggle", () => {
      if (preview.open) {
        expandedArtifacts.add(previewKey);
        loadPreview();
      } else {
        expandedArtifacts.delete(previewKey);
      }
    });
    group.append(toolbar, preview);
    row.append(group);
  }

  function selectedInputDescription(step, slot, record) {
    const configured = record.config.steps.find(candidate => candidate.id === step.id);
    const reference = configured?.inputs[slot];
    if (reference?.source === "opened") return inputSpecs[reference.id]?.label ?? reference.id;
    if (reference?.source === "step") {
      const producer = record.config.steps.find(candidate => candidate.id === reference.stepId);
      return producer ? labels[producer.type] + " · " + (outputLabels[reference.output] ?? reference.output) : reference.output;
    }
    return "Geen invoer";
  }

  function renderRecord(record) {
    const stale = isStale(record);
    const group = make("section", undefined, "stack");
    group.append(make("h3", record.manual ? "Losse stap" : "Pipelinerun"));
    for (const step of record.run.steps) {
      const row = make("article", undefined, "result-row");
      row.dataset.stepId = step.id;
      row.dataset.stepType = step.type;
      row.dataset.status = step.status;
      const header = make("div", undefined, "row between");
      header.append(make("strong", labels[step.type]));
      const states = make("div", undefined, "row");
      states.append(badge(statusLabels[step.status] ?? step.status, step.status));
      if (stale) states.append(badge("Verouderd", "stale"));
      header.append(states);
      row.append(header);
      const slots = Object.keys(core.steps[step.type].inputs);
      row.append(make("p", slots.map(slot =>
        (inputLabels[slot] ?? slot) + " · " + (step.inputs?.[slot]?.name ?? selectedInputDescription(step, slot, record))
      ).join("  |  "), "subtle"));
      if (step.message) row.append(make("p", step.message, "subtle"));
      if (step.diagnostics?.length) {
        const list = make("ul", undefined, "diagnostics");
        renderDiagnostics(list, step.diagnostics);
        row.append(list);
      }
      for (const [key, artifact] of Object.entries(step.outputs ?? {})) {
        addArtifact(row, key, artifact, [record.manual ? "manual" : "pipeline", step.id, key].join("/"));
      }
      group.append(row);
    }
    results.append(group);
  }

  function displayedRecords() {
    return [inProgress?.manual ? lastPipeline : inProgress ?? lastPipeline, inProgress?.manual ? inProgress : lastManual]
      .filter(record => record?.run.steps.length);
  }

  function isStale(record) {
    return record.revision !== (record.manual ? manualRevision : revision);
  }

  function auditRecords(records = displayedRecords()) {
    return records.flatMap(record => (record.run.audit ?? []).map(diagnostic => ({
      ...diagnostic,
      stale: isStale(record),
    })));
  }

  function renderResults() {
    const records = displayedRecords();
    results.replaceChildren();
    for (const record of records) renderRecord(record);
    byId("results-empty").hidden = records.length > 0;
    byId("results-stale").hidden = !records.some(isStale);
    byId("result-tools").hidden = !records.some(record => record.run.steps.some(step => step.diagnostics?.length));
    const audit = auditRecords(records);
    byId("merged-audit").hidden = audit.length === 0;
    renderDiagnostics(byId("merged-gaps-list"), audit, true);
  }

  function applyControls() {
    stopOnError.checked = config.errorPolicy === "stop";
    const converters = config.steps.filter(step => step.type === "convert");
    if (converters.length) {
      converterValidation.checked = converters.every(step => step.options?.validate !== false);
      converterValidation.indeterminate = !converterValidation.checked && converters.some(step => step.options?.validate !== false);
    }
    const checks = config.steps.filter(step => step.type === "check");
    if (checks.length) {
      for (const control of checkboxes) {
        const enabled = checks.map(step => (step.options?.kCodes ?? core.kCodes).includes(control.value));
        control.checked = enabled.every(Boolean);
        control.indeterminate = !control.checked && enabled.some(Boolean);
      }
    }
  }

  function bindPipeline() {
    mode.addEventListener("change", () => {
      if (currentMode === "custom") savedCustom = config;
      currentMode = mode.value;
      if (mode.value === "custom") {
        config = savedCustom ?? cloneConfig(config);
      } else {
        config = makePreset(mode.value);
      }
      applyControls();
      invalidate({ steps: true, scope: "pipeline" });
    });
    verify.addEventListener("change", () => {
      const updated = makePreset(mode.value);
      for (const step of updated.steps) {
        const existing = config.steps.find(candidate => candidate.id === step.id && candidate.type === step.type);
        if (existing) {
          step.inputs = existing.inputs;
          step.options = existing.options;
        }
      }
      config = updated;
      invalidate({ steps: true, scope: "pipeline" });
    });
    stopOnError.addEventListener("change", () => {
      config.errorPolicy = stopOnError.checked ? "stop" : "continue";
      invalidate({ manual: false, scope: "pipeline" });
    });
    converterValidation.addEventListener("change", () => {
      for (const step of [...config.steps, manualStep].filter(step => step.type === "convert")) {
        step.options.validate = converterValidation.checked;
      }
      invalidate({ manual: false, steps: true });
    });
    for (const control of checkboxes) {
      control.addEventListener("change", () => {
        for (const step of [...config.steps, manualStep].filter(step => step.type === "check")) {
          updateKCode(step, control.value, control.checked);
        }
        invalidate({ manual: false, steps: true });
      });
    }
    byId("pipeline-add").addEventListener("click", () => {
      const step = core.newStep(addStep.value, newId());
      applyDefaultOptions(step);
      for (const [slot, declaration] of Object.entries(core.steps[step.type].inputs)) {
        if (originals[step.inputs[slot]?.id]) continue;
        const choices = compatibleChoices(declaration, config.steps);
        const alternative = choices.find(choice => choice.ref.source === "opened" && originals[choice.ref.id])
          ?? choices.findLast(choice => choice.ref.source === "step");
        if (alternative) step.inputs[slot] = alternative.ref;
      }
      config.steps.push(step);
      invalidate({ steps: true, scope: "pipeline" });
      stepList.lastElementChild?.querySelector("select")?.focus();
    });
    pipelineRun.addEventListener("click", runPipeline);
    byId("manual-run").addEventListener("click", runManual);
    manualOperation.addEventListener("change", () => {
      manualStep = core.newStep(manualOperation.value, newId());
      applyDefaultOptions(manualStep);
      invalidate({ scope: "manual" });
    });
    byId("pipeline-config-export").addEventListener("click", () => {
      try {
        download("uagl-pipeline.json", core.serializeConfig(config), "application/json");
        status("config-status", "Configuratie geëxporteerd.");
      } catch (error) {
        status("config-status", error.message, true);
      }
    });
    byId("pipeline-config-file").addEventListener("change", async event => {
      const file = event.target.files?.[0];
      if (!file) return;
      try {
        const text = new TextDecoder("utf-8").decode(await file.arrayBuffer());
        config = core.parseConfig(text);
        mode.value = "custom";
        currentMode = "custom";
        savedCustom = config;
        applyControls();
        invalidate({ steps: true, scope: "pipeline" });
        status("config-status", "Configuratie geladen. Koppel de bronbestanden bij Invoer.");
      } catch (error) {
        status("config-status", error.message, true);
      } finally {
        event.target.value = "";
      }
    });
    byId("diagnostic-filter").addEventListener("input", renderResults);
    byId("k-filter").addEventListener("change", renderResults);
    byId("merged-gaps-export").addEventListener("click", () => {
      download("uagldiagnoses.json", { version: 1, diagnostics: auditRecords() }, "application/json");
    });
  }

  for (const type of Object.keys(core.steps)) {
    for (const control of [addStep, manualOperation]) {
      const option = make("option", labels[type]);
      option.value = type;
      control.append(option);
    }
  }
  manualOperation.value = "assemble";
  config = makePreset(mode.value);
  manualStep = core.newStep(manualOperation.value, newId());
  applyDefaultOptions(manualStep);
  bindInput();
  bindPipeline();
  showInput();
  renderSteps();
  renderManualInputs();
  renderPreflight();
})();
