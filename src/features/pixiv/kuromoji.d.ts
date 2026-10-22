declare module "kuromoji/src/dict/DynamicDictionaries.js" {
  export default class DynamicDictionaries {
    loadTrie(base: Int32Array, check: Int32Array): void;
    loadTokenInfoDictionaries(info: Uint8Array, pos: Uint8Array, map: Uint8Array): void;
    loadConnectionCosts(costs: Int16Array): void;
    loadUnknownDictionaries(info: Uint8Array, pos: Uint8Array, map: Uint8Array, chars: Uint8Array, compat: Uint32Array, invoke: Uint8Array): void;
  }
}
declare module "kuromoji/src/Tokenizer.js" {
  import DynamicDictionaries from "kuromoji/src/dict/DynamicDictionaries.js";
  export default class Tokenizer {
    constructor(dictionary: DynamicDictionaries);
    tokenize(text: string): { surface_form: string; reading?: string; pronunciation?: string }[];
  }
}
