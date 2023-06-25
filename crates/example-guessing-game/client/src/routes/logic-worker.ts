const wasmAddr = '/release.wasm';
const inputCaps = 4096;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

// const memory = new WebAssembly.Memory({ initial: 32, maximum: 1024 });
let memory: WebAssembly.Memory | undefined;
const memSlice = (ptr: number, len: number): string => {
	if (!memory) {
		console.error('WASM Memory not initialized');
		return '';
	}
	return decoder.decode(new Uint8Array(memory.buffer, ptr, len));
};

const shmem = new SharedArrayBuffer(inputCaps + Int32Array.BYTES_PER_ELEMENT);
const inputArray = new Uint8Array(shmem);
const lock = new Int32Array(shmem, inputCaps, 1);

async function main() {
	const { instance } = await WebAssembly.instantiateStreaming(fetch(wasmAddr), {
		env: {
			rulebook_trigger_io(paramPtr: number): number {
				if (!memory) {
					console.error('WASM Memory not initialized');
					return 0;
				}
				console.log(`MEM: len:${memory.buffer.byteLength}`);
				const [inputPtr, inputCap, outputPtr, outputLen] = new Int32Array(
					memory.buffer,
					paramPtr,
					4
				);
				const output = JSON.parse(memSlice(outputPtr, outputLen));
				console.log(`IO: output:`, output);

				Atomics.store(lock, 0, 0);
				self.postMessage({ type: 'io', output });
				const waitRes = Atomics.wait(lock, 0, 0);
				console.log(`lock from worker: ${waitRes}, len: ${lock[0]}`);

				if (lock[0] > inputCap) {
					throw new Error(`input overflow: ${lock[0]}`);
				}
				let lhs = new Uint8Array(memory.buffer, inputPtr, lock[0]);
				let rhs = new Uint8Array(shmem, 0, lock[0]);
				lhs.set(rhs);
				return lock[0];
			},
			rulebook_log(msgPtr: number, msgLen: number) {
				console.log(`LOG: ${memSlice(msgPtr, msgLen)}`);
			}
		}
	});
	memory = instance.exports.memory as WebAssembly.Memory;

	self.postMessage({ type: 'shmem', lock, inputArray });

	const start = instance.exports.rulebook_start_session as (
		inputCaps: number,
		printState: number
	) => void;
	start(inputCaps, 1);
}

main();
