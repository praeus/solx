/**
 * React Query hooks wrapping the api.ts fetchers.
 *
 * Hook naming convention:
 *   useFoo()        — query hook
 *   useFooMutation  — mutation hook
 *
 * Each hook exposes React Query's full surface (data, isLoading, error, refetch…)
 * so callers don't need to import useQuery/useMutation directly.
 */

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
  fetchConfig,
  fetchDocument,
  fetchDocuments,
  fetchFileContent,
  fetchHealth,
  fetchType,
  fetchTypes,
  listFiles,
  patchConfig,
  searchActions,
  searchDocuments,
  uploadFile,
  type ActionExecResult,
  type ActionSummary,
  type DocumentSummary,
  type FileContentResponse,
  type FileListResponse,
  type HealthResponse,
  type Page,
  type SearchResults,
  type SolxConfig,
  type TypeEntity,
} from "./api";

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
  path_prefix?: string;
  limit?: number;
  offset?: number;
  sort_by?: string;
  sort_order?: "asc" | "desc";
  enabled?: boolean;
}

export function useDocuments(opts: UseDocumentsOptions = {}): UseQueryResult<Page<DocumentSummary>> {
  return useQuery({
    queryKey: ["documents", "list", opts],
    queryFn: () => fetchDocuments(opts),
    enabled: opts.enabled !== false,
    placeholderData: (previous) => previous,
  });
}

export interface UseSearchOptions {
  q?: string;
  path_prefix?: string;
  type_ref?: string;
  limit?: number;
  offset?: number;
  enabled?: boolean;
}

export function useSearchDocuments(opts: UseSearchOptions): UseQueryResult<SearchResults> {
  return useQuery({
    queryKey: ["documents", "search", opts],
    queryFn: () => searchDocuments(opts),
    enabled: opts.enabled !== false && Boolean(opts.q?.trim()),
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

export function useDocumentTypes(limit = 200): UseQueryResult<Page<TypeEntity>> {
  return useQuery({
    queryKey: ["types", "list", { limit }],
    queryFn: () => fetchTypes({ limit }),
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
  path_prefix?: string;
  limit?: number;
  offset?: number;
  sort_by?: string;
  sort_order?: "asc" | "desc";
  enabled?: boolean;
}

export function useTypes(
  opts: UseTypesBrowseOptions = {},
): UseQueryResult<Page<TypeEntity>> {
  return useQuery({
    queryKey: ["types", "browse", opts],
    queryFn: () => {
      const params: Record<string, string | number | undefined> = {};
      if (opts.path_prefix) params.path_prefix = opts.path_prefix;
      if (opts.limit) params.limit = opts.limit;
      if (opts.offset) params.offset = opts.offset;
      if (opts.sort_by) params.sort_by = opts.sort_by;
      if (opts.sort_order) params.sort_order = opts.sort_order;
      return fetchTypes(params);
    },
    enabled: opts.enabled !== false,
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
  path_prefix?: string;
  limit?: number;
  offset?: number;
  sort_by?: string;
  sort_order?: "asc" | "desc";
  enabled?: boolean;
}

export function useActions(opts: UseActionsOptions = {}): UseQueryResult<Page<ActionSummary>> {
  return useQuery({
    queryKey: ["actions", "list", opts],
    queryFn: () => fetchActions(opts),
    enabled: opts.enabled !== false,
    placeholderData: (previous) => previous,
  });
}

export interface UseSearchActionsOptions {
  q?: string;
  path_prefix?: string;
  limit?: number;
  offset?: number;
  enabled?: boolean;
}

export function useSearchActions(opts: UseSearchActionsOptions): UseQueryResult<Page<ActionSummary>> {
  return useQuery({
    queryKey: ["actions", "search", opts],
    queryFn: () => searchActions(opts),
    enabled: (opts.enabled ?? true) && Boolean(opts.q && opts.q.trim()),
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

/** ── Config ─────────────────────────────────────────────────────────── */

export function useConfig(): UseQueryResult<SolxConfig> {
  return useQuery({
    queryKey: ["config"],
    queryFn: fetchConfig,
    staleTime: 60_000,
  });
}

export function useMutateConfig() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (patch: Record<string, unknown>) => patchConfig(patch),
    onSuccess: (data) => {
      qc.setQueryData(["config"], data);
    },
  });
}

/** ── Re-exports ─────────────────────────────────────────────────────── */

export { ApiError };
export type { FileListResponse };
