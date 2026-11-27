package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"testing"

	"github.com/openai/openai-go/v3"
)

type fixtureStream struct {
	chunks []openai.ChatCompletionChunk
	next   int
	err    error
}

func (f *fixtureStream) Next() bool {
	if f.next == len(f.chunks) {
		return false
	}
	f.next++
	return true
}
func (f *fixtureStream) Current() openai.ChatCompletionChunk { return f.chunks[f.next-1] }
func (f *fixtureStream) Err() error                          { return f.err }
func TestStreamRequiresModelCompletion(t *testing.T) {
	for _, tc := range []struct {
		name, finish string
		err          error
		wantDone     bool
	}{
		{"normal stop", "stop", nil, true},
		{"token limit", "length", nil, true},
		{"early EOF", "", nil, false},
		{"filtered", "content_filter", nil, false},
		{"tool call", "tool_calls", nil, false},
		{"transport error after final delta", "length", errors.New("connection lost"), false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			stream := &fixtureStream{err: tc.err, chunks: []openai.ChatCompletionChunk{
				{Choices: []openai.ChatCompletionChunkChoice{{Delta: openai.ChatCompletionChunkChoiceDelta{Content: "draft"}}}},
				{Choices: []openai.ChatCompletionChunkChoice{{FinishReason: tc.finish, Delta: openai.ChatCompletionChunkChoiceDelta{Content: " text"}}}},
			}}
			var out bytes.Buffer
			err := writeAnswer(stream, json.NewEncoder(&out))
			if (err == nil) != tc.wantDone {
				t.Fatalf("err %v, frames %s", err, &out)
			}
			if tc.wantDone {
				if want := "{\"delta\":\"draft\"}\n{\"delta\":\" text\"}\n{\"done\":true}\n"; out.String() != want {
					t.Fatalf("unexpected sequence of frames: %s", &out)
				}
			} else if bytes.Contains(out.Bytes(), []byte(`"done":true`)) {
				t.Fatalf("incomplete stream emitted a success frame: %s", &out)
			}
		})
	}
}
