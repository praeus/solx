/**
 * React Query hooks wrapping the api.ts fetchers.
 *
 * Hook naming convention:
 *   useFoo()                          — query hook
 *   useCreateFoo / useUpdateFoo / …   — mutation hook, named after the verb
 *
 * Each hook exposes React Query's full surface (data, isLoading, error, refetch…)
 * so callers don't need to import useQuery/useMutation directly.
 */

import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient, type UseQueryResult } from "@tanstack/react-query";
import {
  ApiError,
  createAction,
  createDocument,
  createType,
  deleteAction,
  deleteDocument,
  deleteFile,
  deleteType,
  execAction,
  fetchAction,
  fetchActions,
  fetchDocument,
  fetchDocuments,
  fetchFileContent,
  fetchHealth,
  fetchType,
  fetchTypes,
  listFiles,
  searchDocuments,
  updateDocument,
  uploadFile,
  type ActionExecResult,
  type ActionSummary,
  type DocumentSummary,
  type FileContentResponse,
  type FileListResponse,
  type HealthResponse,
  type Page,
  type SearchResults,
  type TypeEntity,
} from "./api";
import { acquireObjectUrl, releaseObjectUrl } from "./objectUrlCache";

/** ── Health ─────────────────────────────────────────────────────────── */

export function useHealth(): UseQueryResult<HealthResponse> {
  return useQuery({
    queryKey: ["health"],
    queryFn: fetchHealth,
    refetchInterval: 15_000,
    retry: false,
  });
}

/** ── Documents ──────────────────────────────────────────────────────── */

export interface UseDocumentsOptions {
  pathPrefix?: string;
  limit?: number;
  offset?: number;
  sortBy?: string;
  sortOrder?: "asc" | "desc";
  /** LIKE filter on a column — typeRef | author | title | summary | name. */
  filterField?: string;
  filterValue?: string;
  enabled?: boolean;
}

export function useDocuments(opts: UseDocumentsOptions = {}): UseQueryResult<Page<DocumentSummary>> {
  const { enabled, ...params } = opts;
  return useQuery({
    queryKey: ["documents", "list", params],
    queryFn: () => fetchDocuments(params),
    enabled: enabled !== false,
    placeholderData: (previous) => previous,
  });
}

export interface UseSearchOptions {
  q?: string;
  pathPrefix?: string;
  typeRef?: string;
  limit?: number;
  offset?: number;
  enabled?: boolean;
}

export function useSearchDocuments(opts: UseSearchOptions): UseQueryResult<SearchResults> {
  const { enabled, ...params } = opts;
  return useQuery({
    queryKey: ["documents", "search", params],
    queryFn: () => searchDocuments(params),
    enabled: enabled !== false && Boolean(params.q?.trim()),
    placeholderData: (previous) => previous,
  });
}

export function useDocument(path: string | undefined, name: string | undefined): UseQueryResult<DocumentSummary> {
  return useQuery({
    queryKey: ["documents", "item", path, name],
    queryFn: () => fetchDocument(path!, name!),
    enabled: Boolean(path) && Boolean(name),
  });
}

export function useCreateDocument() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createDocument,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["documents"] });
    },
  });
}

export function useUpdateDocument() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      path,
      name,
      input,
    }: {
      path: string;
      name: string;
      input: Parameters<typeof updateDocument>[2];
    }) => updateDocument(path, name, input),
    onSuccess: (_data, vars) => {
      void qc.invalidateQueries({ queryKey: ["documents"] });
      void qc.invalidateQueries({ queryKey: ["documents", "item", vars.path, vars.name] });
    },
  });
}

export function useDeleteDocument() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ path, name }: { path: string; name: string }) => deleteDocument(path, name),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["documents"] });
    },
  });
}

/** ── Types ──────────────────────────────────────────────────────────── */

export function useDocumentTypes(
  limit = 200,
  opts: { groups?: string } = {},
): UseQueryResult<Page<TypeEntity>> {
  const { groups } = opts;
  return useQuery({
    queryKey: ["types", "list", { limit, groups }],
    queryFn: () =>
      fetchTypes(
        groups ? { limit, filterField: "groups", filterValue: groups } : { limit },
      ),
    staleTime: 5 * 60 * 1000,
  });
}

export interface UseTypeOptions {
  enabled?: boolean;
}

export function useType(
  path: string | undefined,
  name: string | undefined,
  opts: UseTypeOptions = {},
): UseQueryResult<TypeEntity> {
  return useQuery({
    queryKey: ["types", "item", path, name],
    queryFn: () => fetchType(path!, name!),
    enabled: (opts.enabled ?? true) && Boolean(path) && Boolean(name),
  });
}

export function useCreateType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createType,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["types"] });
    },
  });
}

export interface UseTypesBrowseOptions {
  pathPrefix?: string;
  limit?: number;
  offset?: number;
  sortBy?: string;
  sortOrder?: "asc" | "desc";
  /** LIKE filter on a column — name | description | groups. */
  filterField?: string;
  filterValue?: string;
  enabled?: boolean;
}

export function useTypes(
  opts: UseTypesBrowseOptions = {},
): UseQueryResult<Page<TypeEntity>> {
  const { enabled, ...params } = opts;
  return useQuery({
    queryKey: ["types", "browse", params],
    queryFn: () => fetchTypes(params),
    enabled: enabled !== false,
    placeholderData: (previous) => previous,
  });
}

export function useDeleteType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ path, name }: { path: string; name: string }) => deleteType(path, name),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["types"] });
    },
  });
}

/** ── Actions ────────────────────────────────────────────────────────── */

export interface UseActionsOptions {
  pathPrefix?: string;
  limit?: number;
  offset?: number;
  sortBy?: string;
  sortOrder?: "asc" | "desc";
  /**
   * LIKE filter on a column — name | caption | description | category |
   * capabilities | phrases | action_type | param_type_ref | result_type_ref |
   * fn_name | bin_name. `action_config` is intentionally not filterable.
   */
  filterField?: string;
  filterValue?: string;
  enabled?: boolean;
}

export function useActions(opts: UseActionsOptions = {}): UseQueryResult<Page<ActionSummary>> {
  const { enabled, ...params } = opts;
  return useQuery({
    queryKey: ["actions", "list", params],
    queryFn: () => fetchActions(params),
    enabled: enabled !== false,
    placeholderData: (previous) => previous,
  });
}

export function useAction(
  path: string | undefined,
  name: string | undefined,
): UseQueryResult<ActionSummary> {
  return useQuery({
    queryKey: ["actions", "item", path, name],
    queryFn: () => fetchAction(path!, name!),
    enabled: Boolean(path) && Boolean(name),
  });
}

export function useCreateAction() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: createAction,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["actions"] });
    },
  });
}

export function useDeleteAction() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ path, name }: { path: string; name: string }) => deleteAction(path, name),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["actions"] });
    },
  });
}

export function useExecAction() {
  return useMutation<
    ActionExecResult,
    Error,
    { path: string; name: string; params?: unknown }
  >({
    mutationFn: ({ path, name, params }) => execAction(path, name, params ?? null),
  });
}

/** ── Files ──────────────────────────────────────────────────────────── */

export function useFilesList(prefix: string, enabled = true): UseQueryResult<FileListResponse> {
  return useQuery({
    queryKey: ["files", "list", { prefix }],
    queryFn: () => listFiles(prefix),
    enabled,
    placeholderData: (previous) => previous,
  });
}

export function useFileContent(
  relPath: string | undefined,
  enabled = true,
): UseQueryResult<FileContentResponse> {
  return useQuery({
    queryKey: ["files", "content", relPath],
    queryFn: () => fetchFileContent(relPath!),
    enabled: enabled && Boolean(relPath),
    staleTime: 60_000,
  });
}

export function useUploadFile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ relPath, bytes }: { relPath: string; bytes: Uint8Array }) =>
      uploadFile(relPath, bytes),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["files"] });
    },
  });
}

export function useDeleteFile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (relPath: string) => deleteFile(relPath),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["files"] });
    },
  });
}

/** ── Files: object URLs (blob: preview for images etc.) ────────────────
 *
 * Replaces the old Bun backend's `/api/files/raw?relPath=…` route, which
 * served bytes directly with a `Content-Type` header — `solx-server` only
 * exposes files as base64 JSON, and the token can't ride along on a plain
 * `<img src>` URL. See `objectUrlCache.ts`.
 */

export function useFileObjectUrl(relPath: string | undefined): string | undefined {
  const [url, setUrl] = useState<string | undefined>(undefined);

  useEffect(() => {
    if (!relPath) {
      setUrl(undefined);
      return;
    }
    let cancelled = false;
    setUrl(undefined);
    acquireObjectUrl(relPath).then((resolved) => {
      if (!cancelled) setUrl(resolved);
    });
    return () => {
      cancelled = true;
      releaseObjectUrl(relPath);
    };
  }, [relPath]);

  return url;
}

/** ── Re-exports ─────────────────────────────────────────────────────── */

export { ApiError };
export type { FileListResponse };
