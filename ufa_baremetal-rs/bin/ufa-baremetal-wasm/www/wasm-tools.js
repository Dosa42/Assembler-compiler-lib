/* Offline bindings for the converter, assembler, and checker Wasm modules. */
globalThis.WasmTools = (() => {
  "use strict";

  const encoder = new TextEncoder();
  const decoder = new TextDecoder("utf-8", { fatal: true });

  function fromBase64(payload) {
    const binary = atob(payload);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  }

  function utf8(input) {
    if (typeof input === "string") return encoder.encode(input);
    if (ArrayBuffer.isView(input) && input.BYTES_PER_ELEMENT === 1) {
      return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
    }
    throw new TypeError("Expected text or input bytes.");
  }

  function callJson(wasm, operation, inputs, options) {
    const buffers = inputs.map((bytes) => {
      const length = bytes.byteLength;
      const pointer = wasm.alloc(length) >>> 0;
      new Uint8Array(wasm.memory.buffer, pointer, length).set(bytes);
      return [pointer, length];
    });

    const packed = BigInt.asUintN(64, wasm[operation](...buffers.flat(), ...options));
    const pointer = Number(packed >> 32n);
    const length = Number(packed & 0xffff_ffffn);
    const text = decoder.decode(new Uint8Array(wasm.memory.buffer, pointer, length));

    wasm.dealloc(pointer, length);
    for (const input of buffers.reverse()) wasm.dealloc(...input);
    return JSON.parse(text);
  }

  function createModule(payload, label) {
    let ready;
    const load = async () => {
      const { instance } = await WebAssembly.instantiate(fromBase64(payload));
      return instance.exports;
    };

    return async (operation, inputs, ...options) => {
      let pending;
      let result;
      try {
        // Another caller may have discarded the instance after it trapped.
        do {
          pending = ready ??= load();
          const wasm = await pending;
          if (pending === ready) {
            result = callJson(wasm, operation, inputs, options);
            break;
          }
        } while (true);
      } catch (cause) {
        // Discard the whole instance and its buffers after a trap or ABI fault.
        // Freeing buffers in a trapped instance can itself trap.
        if (ready === pending) ready = undefined;
        throw new Error(`${label}: ${cause.message ?? String(cause)}`, { cause });
      }
      if (result && typeof result.error === "string") throw new Error(result.error);
      return result;
    };
  }

  function outputPath(sourcePath) {
    const filename = sourcePath.split("/").filter(Boolean).at(-1) ?? sourcePath;
    const extension = filename.lastIndexOf(".");
    const stem = extension > 0 ? filename.slice(0, extension) : filename;
    return `${stem}.uagl`;
  }

  function create({ converter, assembler, checker }) {
    const convertModule = createModule(converter, "C ↔ UAGL converter");
    const assemblerModule = createModule(assembler, "UAGL assembler");
    const checkerModule = createModule(checker, "Assembly checker");
    const convertRequest = (operation, request) => {
      return convertModule(operation, [encoder.encode(JSON.stringify(request))]);
    };

    return {
      convert(source, filename, validate = true) {
        return convertRequest("convert_c", {
          source,
          source_path: filename,
          output_path: outputPath(filename),
          validate,
        });
      },
      reverse(uagl) {
        return convertRequest("uagl_to_c", { uagl });
      },
      fidelity(c, uagl, cName, uaglName) {
        return convertRequest("fidelity_report", { c_path: cName, c, uagl_path: uaglName, uagl });
      },
      validate(input, filename) {
        return assemblerModule("validate", [utf8(input), encoder.encode(filename)]);
      },
      assemble(input, filename) {
        return assemblerModule("assemble", [utf8(input), encoder.encode(filename)]);
      },
      check(input, filename, mask = 0x3f) {
        return checkerModule("check_selected", [utf8(input), encoder.encode(filename)], mask);
      },
    };
  }

  return Object.freeze({ create });
})();
