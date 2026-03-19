package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

const apiURL = "http://localhost:15702"

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintln(os.Stderr, `usage: endless-cli <command> [key:value...]

BRP commands (prepends endless/ automatically, all use key:value params):
  get_summary                                   game state
  get_perf                                      FPS, UPS, timings
  get_entity <entity>                           inspect entity (single positional arg)
  get_squad index:0                             inspect squad
  list_buildings town:0                         list buildings
  list_npcs town:0 job:Woodcutter               list NPCs
  create_building town:1 kind:Farm row:-5 col:0 place building
  delete_building town:1 row:-5 col:0           remove building
  apply_upgrade town:1 upgrade_idx:0            apply upgrade
  set_time paused:false time_scale:4.0          control time
  set_squad_target squad:13 x:6944 y:11488      move squad
  set_policy town:1 eat_food:true               set town policies
  send_chat town:1 to:0 message:hi friend       send chat (spaces ok)
  set_ai_manager town:1 active:true             configure AI
  recruit_squad town:1                          recruit squad
  dismiss_squad squad:0                         dismiss squad

Tools:
  test                                          baseline BRP test suite
  loop                                          background state poller (10s)
  launch                                        start LLM player Claude session`)
		os.Exit(1)
	}

	cmd := os.Args[1]
	args := os.Args[2:]

	var err error
	switch cmd {
	case "test":
		err = runTest()
	case "loop":
		err = runLoop()
	case "launch":
		err = runLaunch()
	case "get_entity":
		// single positional param: endless-cli get_entity <entity>
		if len(args) < 1 {
			err = fmt.Errorf("usage: endless-cli get_entity <entity>")
		} else {
			err = callAndPrint("endless/get_entity", map[string]interface{}{"entity": args[0]})
		}
	default:
		// everything else: prepend endless/ and pass key:value params
		method := cmd
		if !strings.Contains(method, "/") {
			method = "endless/" + method
		}
		params, perr := parseToonParams(args)
		if perr != nil {
			err = perr
		} else {
			err = callAndPrint(method, params)
		}
	}

	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

// --- JSON-RPC ---

type rpcRequest struct {
	JSONRPC string      `json:"jsonrpc"`
	Method  string      `json:"method"`
	Params  interface{} `json:"params"`
	ID      int         `json:"id"`
}

type rpcResponse struct {
	Result json.RawMessage `json:"result"`
	Error  *struct {
		Message string `json:"message"`
	} `json:"error"`
}

func rpc(method string, params map[string]interface{}) (string, error) {
	if params == nil {
		params = map[string]interface{}{}
	}
	body, _ := json.Marshal(rpcRequest{JSONRPC: "2.0", Method: method, Params: params, ID: 1})
	resp, err := http.Post(apiURL, "application/json", bytes.NewReader(body))
	if err != nil {
		return "", fmt.Errorf("connect to %s: %w", apiURL, err)
	}
	defer resp.Body.Close()

	var rr rpcResponse
	if err := json.NewDecoder(resp.Body).Decode(&rr); err != nil {
		return "", fmt.Errorf("decode response: %w", err)
	}
	if rr.Error != nil {
		return "", fmt.Errorf("rpc error: %s", rr.Error.Message)
	}

	var s string
	if err := json.Unmarshal(rr.Result, &s); err == nil {
		return s, nil
	}
	var buf bytes.Buffer
	json.Indent(&buf, rr.Result, "", "  ")
	return buf.String(), nil
}

func callAndPrint(method string, params map[string]interface{}) error {
	result, err := rpc(method, params)
	if err != nil {
		return err
	}
	fmt.Println(result)
	return nil
}

// --- TOON param parsing ---

func parseToonValue(s string) interface{} {
	if s == "true" {
		return true
	}
	if s == "false" {
		return false
	}
	if s == "null" {
		return nil
	}
	if n, err := strconv.Atoi(s); err == nil {
		return n
	}
	if f, err := strconv.ParseFloat(s, 64); err == nil {
		return f
	}
	return s
}

func parseToonParams(args []string) (map[string]interface{}, error) {
	if len(args) == 0 {
		return nil, nil
	}
	if len(args) == 1 && strings.HasPrefix(args[0], "{") {
		var m map[string]interface{}
		return m, json.Unmarshal([]byte(args[0]), &m)
	}
	params := make(map[string]interface{})
	var lastKey string
	for _, arg := range args {
		idx := strings.Index(arg, ":")
		if idx < 0 {
			// no colon -- append to previous key's value (supports spaces in values)
			if lastKey != "" {
				params[lastKey] = fmt.Sprintf("%v %s", params[lastKey], arg)
			} else {
				return nil, fmt.Errorf("bad param (expected key:value): %s", arg)
			}
			continue
		}
		lastKey = arg[:idx]
		params[lastKey] = parseToonValue(arg[idx+1:])
	}
	return params, nil
}

// --- test ---

func runTest() error {
	fmt.Println("waiting for BRP...")
	for i := 0; i < 30; i++ {
		_, err := rpc("endless/get_perf", nil)
		if err == nil {
			fmt.Printf("BRP ready after %ds\n\n", i)
			break
		}
		if i == 29 {
			return fmt.Errorf("BRP not responding after 15s")
		}
		time.Sleep(500 * time.Millisecond)
	}

	fmt.Println("=== PERF ===")
	if err := callAndPrint("endless/get_perf", nil); err != nil {
		fmt.Printf("FAIL: %v\n", err)
	} else {
		fmt.Println("PASS")
	}

	fmt.Println("\n=== SUMMARY ===")
	if err := callAndPrint("endless/get_summary", nil); err != nil {
		fmt.Printf("FAIL: %v\n", err)
	} else {
		fmt.Println("PASS")
	}

	return nil
}

// --- loop ---

func runLoop() error {
	exePath, _ := os.Executable()
	logPath := filepath.Join(filepath.Dir(exePath), "loop.log")
	if _, err := os.Stat(filepath.Dir(exePath)); err != nil {
		logPath = "loop.log"
	}

	logFile, err := os.Create(logPath)
	if err != nil {
		return fmt.Errorf("create %s: %w", logPath, err)
	}
	defer logFile.Close()

	out := io.MultiWriter(os.Stdout, logFile)
	cycle := 0

	for {
		cycle++
		result, err := rpc("endless/get_summary", nil)
		if err != nil {
			fmt.Fprintf(out, "[cycle %d] error: %v\n", cycle, err)
			time.Sleep(10 * time.Second)
			continue
		}
		fmt.Fprintf(out, "\n%s\nCYCLE %d\n%s\n", strings.Repeat("=", 50), cycle, result)
		time.Sleep(10 * time.Second)
	}
}

// --- launch ---

func runLaunch() error {
	exePath, _ := os.Executable()
	dir := filepath.Dir(exePath)
	promptPath := filepath.Join(dir, "prompt.md")
	if _, err := os.Stat(promptPath); err != nil {
		promptPath = "prompt.md"
	}

	prompt, err := os.ReadFile(promptPath)
	if err != nil {
		return fmt.Errorf("read %s: %w", promptPath, err)
	}

	cmd := exec.Command("claude",
		"--model", "claude-haiku-4-5-20251001",
		"--system-prompt", string(prompt),
		"--allowedTools", "Bash(endless-cli*) Bash(cd*) Read",
	)
	cmd.Dir = dir
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Stdin = os.Stdin
	return cmd.Run()
}
