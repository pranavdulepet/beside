package main

import (
	"bytes"
	"encoding/json"
	"github.com/openai/openai-go/v3"
	"testing"
)

type fixtureStream struct {
	chunks []openai.ChatCompletionChunk
	next   int
}

func (f *fixtureStream) Next() bool {
	if f.next == len(f.chunks) {
		return false
	}
	f.next++
	return true
}
func (f *fixtureStream) Current() openai.ChatCompletionChunk { return f.chunks[f.next-1] }
func (f *fixtureStream) Err() error                          { return nil }
func TestStreamRequiresModelCompletion(t *testing.T) {
	for _, finish := range []string{"", "length", "stop"} {
		stream := &fixtureStream{chunks: []openai.ChatCompletionChunk{
			{Choices: []openai.ChatCompletionChunkChoice{{Delta: openai.ChatCompletionChunkChoiceDelta{Content: "draft"}}}},
		}}
		if finish != "" {
			stream.chunks = append(stream.chunks, openai.ChatCompletionChunk{Choices: []openai.ChatCompletionChunkChoice{{FinishReason: finish}}})
		}
		var out bytes.Buffer
		err := writeAnswer(stream, json.NewEncoder(&out))
		if (err == nil) != (finish == "stop") || bytes.Contains(out.Bytes(), []byte(`"done":true`)) != (finish == "stop") {
			t.Fatalf("finish %q, err %v, frames %s", finish, err, out.String())
		}
	}
}
