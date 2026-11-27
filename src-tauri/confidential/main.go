// The companion supplies text only after the router's attestation has been verified.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/openai/openai-go/v3"
	"github.com/openai/openai-go/v3/option"
	tinfoil "github.com/tinfoilsh/tinfoil-go"
)

type handshake struct{ Op, Key string }
type prompt struct {
	Model, System, Input string
	MaxTokens            int64
}

func run() error {
	deadline := time.AfterFunc(45*time.Second, func() { os.Exit(2) })
	defer deadline.Stop()
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 4096), 2<<20)
	read := func(target any) error {
		if !in.Scan() {
			return fmt.Errorf("missing request")
		}
		return json.Unmarshal(in.Bytes(), target)
	}
	out := json.NewEncoder(os.Stdout)
	var hello handshake
	if read(&hello) != nil || (hello.Op != "verify" && hello.Op != "ask") {
		return fmt.Errorf("invalid operation")
	}
	client, err := tinfoil.NewClientWithOptions(tinfoil.WithTransport(tinfoil.TransportTLS), tinfoil.WithOpenAIOptions(option.WithAPIKey(hello.Key), option.WithMaxRetries(0)))
	if err != nil || client.VerificationDocument() == nil {
		return fmt.Errorf("could not verify the confidential router")
	}
	if err := out.Encode(map[string]bool{"verified": true}); err != nil {
		return err
	}
	if hello.Op == "verify" {
		return nil
	}
	var p prompt
	if read(&p) != nil || p.Model == "" || p.Input == "" || p.MaxTokens < 1 || p.MaxTokens > 32768 {
		return fmt.Errorf("invalid prompt")
	}
	deadline.Reset(90 * time.Second)
	ctx, cancel := context.WithTimeout(context.Background(), 85*time.Second)
	defer cancel()
	stream := client.Chat.Completions.NewStreaming(ctx, openai.ChatCompletionNewParams{
		Model: p.Model, Messages: []openai.ChatCompletionMessageParamUnion{openai.SystemMessage(p.System), openai.UserMessage(p.Input)},
		MaxCompletionTokens: openai.Int(p.MaxTokens),
	})
	defer stream.Close()
	return writeAnswer(stream, out)
}

type completionStream interface {
	Next() bool
	Current() openai.ChatCompletionChunk
	Err() error
}

func writeAnswer(stream completionStream, out *json.Encoder) error {
	finished := false
	for stream.Next() {
		chunk := stream.Current()
		if len(chunk.Choices) == 0 || chunk.Choices[0].Index != 0 {
			continue
		}
		choice := chunk.Choices[0]
		switch choice.FinishReason {
		case "stop", "length":
			finished = true
		case "":
			// An ordinary delta has no finish reason.
		default:
			return fmt.Errorf("the model stopped before completing the answer")
		}
		if choice.Delta.Content != "" {
			if err := out.Encode(map[string]string{"delta": choice.Delta.Content}); err != nil {
				return err
			}
		}
	}
	if stream.Err() != nil || !finished {
		return fmt.Errorf("the confidential model could not complete the response")
	}
	return out.Encode(map[string]bool{"done": true})
}
func main() {
	if err := run(); err != nil {
		json.NewEncoder(os.Stdout).Encode(map[string]string{"error": err.Error()})
		os.Exit(1)
	}
}
