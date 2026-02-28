import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ChatAgent, ChatMessage } from 'shared/types';
import {
  isMentionAllAlias,
  mentionAllAliases,
  mentionAllKeyword,
  mentionRegex,
} from '../constants';
import { extractMentions } from '../utils';

export interface UseMessageInputResult {
  draft: string;
  setDraft: React.Dispatch<React.SetStateAction<string>>;
  selectedMentions: string[];
  setSelectedMentions: React.Dispatch<React.SetStateAction<string[]>>;
  mentionQuery: string | null;
  setMentionQuery: React.Dispatch<React.SetStateAction<string | null>>;
  showMentionAllSuggestion: boolean;
  replyToMessage: ChatMessage | null;
  setReplyToMessage: React.Dispatch<React.SetStateAction<ChatMessage | null>>;
  inputRef: React.RefObject<HTMLTextAreaElement>;
  handleDraftChange: (value: string) => void;
  handleMentionSelect: (name: string) => void;
  handleReplySelect: (message: ChatMessage, mentionHandle: string | null) => void;
  visibleMentionSuggestions: ChatAgent[];
  agentOptions: { value: string; label: string }[];
  resetInput: () => void;
  highlightedMentionIndex: number;
  setHighlightedMentionIndex: React.Dispatch<React.SetStateAction<number>>;
  handleMentionKeyDown: (event: React.KeyboardEvent) => boolean;
}

export function useMessageInput(
  mentionAgents: ChatAgent[]
): UseMessageInputResult {
  const inputRef = useRef<HTMLTextAreaElement>(null!);
  const [draft, setDraft] = useState('');
  const [selectedMentions, setSelectedMentions] = useState<string[]>([]);
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [replyToMessage, setReplyToMessage] = useState<ChatMessage | null>(null);
  const [highlightedMentionIndex, setHighlightedMentionIndex] = useState(0);

  const handleDraftChange = useCallback((value: string) => {
    setDraft(value);
    const match = mentionRegex.exec(value);
    if (match) {
      setMentionQuery(match[2] ?? '');
      // Reset highlight to first item when query changes
      setHighlightedMentionIndex(0);
    } else {
      setMentionQuery(null);
      setHighlightedMentionIndex(0);
    }
  }, []);

  const handleMentionSelect = useCallback((name: string) => {
    setDraft((prev) => {
      const match = mentionRegex.exec(prev);
      if (!match) {
        return `${prev}${prev.endsWith(' ') || prev.length === 0 ? '' : ' '}@${name} `;
      }
      const matchIndex = match.index ?? prev.length;
      const prefix = prev.slice(0, matchIndex);
      const beforeAt = match[1] ?? '';
      const needsSeparator = beforeAt.length > 0 && !/\s/u.test(beforeAt);
      const normalizedBeforeAt = needsSeparator ? `${beforeAt} ` : beforeAt;
      return `${prefix}${normalizedBeforeAt}@${name} `;
    });
    setSelectedMentions((prev) =>
      prev.includes(name) ? prev : [...prev, name]
    );
    setMentionQuery(null);
    inputRef.current?.focus();
  }, []);

  const handleReplySelect = useCallback(
    (message: ChatMessage, mentionHandle: string | null) => {
      setReplyToMessage(message);
      if (mentionHandle) {
        setDraft((prev) => {
          const mentions = extractMentions(prev);
          if (mentions.has(mentionHandle)) return prev;
          const prefix = `@${mentionHandle}`;
          if (!prev.trim()) return `${prefix} `;
          return `${prefix} ${prev}`;
        });
        setSelectedMentions((prev) =>
          prev.includes(mentionHandle) ? prev : [...prev, mentionHandle]
        );
        setMentionQuery(null);
      }
      inputRef.current?.focus();
    },
    []
  );

  const visibleMentionSuggestions = useMemo(() => {
    if (mentionQuery === null) return [];
    const query = mentionQuery.toLowerCase();
    return mentionAgents.filter((agent) =>
      agent.name.toLowerCase().includes(query)
    );
  }, [mentionAgents, mentionQuery]);

  const showMentionAllSuggestion = useMemo(() => {
    if (mentionQuery === null) return false;
    const query = mentionQuery.trim().toLowerCase();
    if (!query) return true;
    return mentionAllAliases.some((alias) =>
      alias.toLowerCase().startsWith(query)
    ) || isMentionAllAlias(query) || mentionAllKeyword.startsWith(query);
  }, [mentionQuery]);

  // Handle keyboard navigation for mention suggestions
  // Returns true if the event was handled (should prevent default behavior)
  const handleMentionKeyDown = useCallback(
    (event: React.KeyboardEvent): boolean => {
      // Only handle when mention suggestions are visible
      const totalSuggestionCount =
        visibleMentionSuggestions.length + (showMentionAllSuggestion ? 1 : 0);
      if (mentionQuery === null || totalSuggestionCount === 0) {
        return false;
      }

      if (event.key === 'ArrowDown') {
        event.preventDefault();
        setHighlightedMentionIndex((prev) =>
          prev < totalSuggestionCount - 1 ? prev + 1 : 0
        );
        return true;
      }

      if (event.key === 'ArrowUp') {
        event.preventDefault();
        setHighlightedMentionIndex((prev) =>
          prev > 0 ? prev - 1 : totalSuggestionCount - 1
        );
        return true;
      }

      if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        if (showMentionAllSuggestion && highlightedMentionIndex === 0) {
          handleMentionSelect(mentionAllKeyword);
          return true;
        }
        const agentIndex = highlightedMentionIndex - (showMentionAllSuggestion ? 1 : 0);
        const selectedAgent = visibleMentionSuggestions[agentIndex];
        if (selectedAgent) {
          handleMentionSelect(selectedAgent.name);
        }
        return true;
      }

      if (event.key === 'Escape') {
        event.preventDefault();
        setMentionQuery(null);
        setHighlightedMentionIndex(0);
        return true;
      }

      return false;
    },
    [
      mentionQuery,
      visibleMentionSuggestions,
      showMentionAllSuggestion,
      highlightedMentionIndex,
      handleMentionSelect,
    ]
  );

  const agentOptions = useMemo(
    () =>
      mentionAgents.map((agent) => ({
        value: agent.name,
        label: agent.name,
      })),
    [mentionAgents]
  );

  // Sync selected mentions with available agents
  useEffect(() => {
    if (mentionAgents.length === 0) {
      setSelectedMentions([]);
      return;
    }
    setSelectedMentions((prev) =>
      prev.filter((mention) =>
        mention === mentionAllKeyword ||
        mentionAgents.some((agent) => agent.name === mention)
      )
    );
  }, [mentionAgents]);

  const resetInput = useCallback(() => {
    setDraft('');
    setSelectedMentions([]);
    setMentionQuery(null);
    setReplyToMessage(null);
    setHighlightedMentionIndex(0);
  }, []);

  return {
    draft,
    setDraft,
    selectedMentions,
    setSelectedMentions,
    mentionQuery,
    setMentionQuery,
    showMentionAllSuggestion,
    replyToMessage,
    setReplyToMessage,
    inputRef,
    handleDraftChange,
    handleMentionSelect,
    handleReplySelect,
    visibleMentionSuggestions,
    agentOptions,
    resetInput,
    highlightedMentionIndex,
    setHighlightedMentionIndex,
    handleMentionKeyDown,
  };
}
