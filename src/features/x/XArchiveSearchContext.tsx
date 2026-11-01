import {createContext} from "react";
import type {XArchiveAsset, XArchiveRecord, XCapturePost} from "./xArchiveApi";

/** The caller supplies only an owned manifest asset, never a path or remote URL. */
export type ArchiveSearchAction=(record:XArchiveRecord,post:XCapturePost,asset:XArchiveAsset)=>Promise<void>;
export const ArchiveSearchContext=createContext<ArchiveSearchAction|null>(null);
