// Ambient declarations for the AudioWorkletGlobalScope. These types are not in
// the standard DOM lib; the worklet runs in its own global scope.

declare const sampleRate: number;
declare const currentTime: number;

declare abstract class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor(options?: unknown);
  process(
    inputs: Float32Array[][],
    outputs: Float32Array[][],
    parameters: Record<string, Float32Array>,
  ): boolean;
}

declare function registerProcessor(
  name: string,
  processorCtor: new (options?: unknown) => AudioWorkletProcessor,
): void;
