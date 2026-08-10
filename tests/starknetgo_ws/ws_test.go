package starknetgo_ws

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	"github.com/NethermindEth/juno/core/felt"
	"github.com/NethermindEth/starknet.go/rpc"
)

const waitTimeout = 45 * time.Second

type txHashes struct {
	Hashes []string `json:"hashes"`
}

type probe struct {
	name     string
	required bool
	start    func(context.Context, string) (activeProbe, error)
}

type activeProbe struct {
	name     string
	required bool
	id       string
	await    func(time.Duration) result
	close    func()
}

type result struct {
	name     string
	id       string
	observed bool
	detail   string
	err      error
}

func TestMadaraWebsocketSubscriptions(t *testing.T) {
	httpURL := mustEnv(t, "MADARA_HTTP_URL")
	wsURL := mustEnv(t, "MADARA_WS_URL")
	readyFile := mustEnv(t, "MADARA_READY_FILE")
	statusReadyFile := mustEnv(t, "MADARA_STATUS_READY_FILE")
	txFile := mustEnv(t, "MADARA_TX_FILE")
	account0 := mustFelt(t, mustEnv(t, "MADARA_ACCOUNT_0"))
	account1 := mustFelt(t, mustEnv(t, "MADARA_ACCOUNT_1"))
	erc20 := mustFelt(t, mustEnv(t, "MADARA_ERC20_ADDRESS"))
	transferKey := mustFelt(t, mustEnv(t, "MADARA_TRANSFER_KEY"))

	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()

	provider, err := rpc.NewProvider(ctx, httpURL)
	if err != nil && !errors.Is(err, rpc.ErrIncompatibleVersion) {
		t.Fatalf("NewProvider: %v", err)
	}

	spec, err := provider.SpecVersion(ctx)
	if err != nil {
		t.Fatalf("SpecVersion: %v", err)
	}
	block, err := provider.BlockHashAndNumber(ctx)
	if err != nil {
		t.Fatalf("BlockHashAndNumber: %v", err)
	}
	t.Logf("spec_version=%s block_number=%d block_hash=%s", spec, block.Number, block.Hash)

	latest := new(rpc.SubscriptionBlockID).WithLatestTag()
	currentNumber := new(rpc.SubscriptionBlockID).WithBlockNumber(block.Number)
	currentHash := new(rpc.SubscriptionBlockID).WithBlockHash(block.Hash)

	probes := []probe{
		newHeads("newHeads/default", rpc.SubscriptionBlockID{}, true),
		newHeads("newHeads/block_id=latest", latest, true),
		newHeads("newHeads/block_id=current_number", currentNumber, true),
		newHeads("newHeads/block_id=current_hash", currentHash, true),

		events("events/default", nil, true),
		events("events/finality=PRE_CONFIRMED", &rpc.EventSubscriptionInput{FinalityStatus: rpc.TxnFinalityStatusPreConfirmed}, true),
		events("events/finality=ACCEPTED_ON_L2", &rpc.EventSubscriptionInput{FinalityStatus: rpc.TxnFinalityStatusAcceptedOnL2}, true),
		events("events/block_id=latest", &rpc.EventSubscriptionInput{SubBlockID: latest}, true),
		events("events/block_id=current_number", &rpc.EventSubscriptionInput{SubBlockID: currentNumber}, true),
		events("events/block_id=current_hash", &rpc.EventSubscriptionInput{SubBlockID: currentHash}, true),
		events("events/from_address", &rpc.EventSubscriptionInput{FromAddress: rpc.AddressList{erc20}}, true),
		events("events/key0", &rpc.EventSubscriptionInput{Keys: [][]*felt.Felt{{transferKey}}}, true),
		events("events/from_address+key0", &rpc.EventSubscriptionInput{
			FromAddress: rpc.AddressList{erc20},
			Keys:        [][]*felt.Felt{{transferKey}},
		}, true),

		receipts("receipts/default", nil, true),
		receipts("receipts/finality=PRE_CONFIRMED", &rpc.SubNewTxnReceiptsInput{FinalityStatus: []rpc.TxnFinalityStatus{rpc.TxnFinalityStatusPreConfirmed}}, true),
		receipts("receipts/finality=ACCEPTED_ON_L2", &rpc.SubNewTxnReceiptsInput{FinalityStatus: []rpc.TxnFinalityStatus{rpc.TxnFinalityStatusAcceptedOnL2}}, true),
		receipts("receipts/finality=PRE_CONFIRMED+ACCEPTED_ON_L2", &rpc.SubNewTxnReceiptsInput{
			FinalityStatus: []rpc.TxnFinalityStatus{rpc.TxnFinalityStatusPreConfirmed, rpc.TxnFinalityStatusAcceptedOnL2},
		}, true),
		receipts("receipts/sender=account0", &rpc.SubNewTxnReceiptsInput{SenderAddress: []*felt.Felt{account0}}, true),
		receipts("receipts/sender=account1", &rpc.SubNewTxnReceiptsInput{SenderAddress: []*felt.Felt{account1}}, true),
		receipts("receipts/sender=account0+PRE_CONFIRMED", &rpc.SubNewTxnReceiptsInput{
			FinalityStatus: []rpc.TxnFinalityStatus{rpc.TxnFinalityStatusPreConfirmed},
			SenderAddress:  []*felt.Felt{account0},
		}, true),
		receipts("receipts/sender=account0+ACCEPTED_ON_L2", &rpc.SubNewTxnReceiptsInput{
			FinalityStatus: []rpc.TxnFinalityStatus{rpc.TxnFinalityStatusAcceptedOnL2},
			SenderAddress:  []*felt.Felt{account0},
		}, true),

		txns("transactions/default", nil, true),
		txns("transactions/finality=RECEIVED", &rpc.SubNewTxnsInput{FinalityStatus: []rpc.TxnStatus{rpc.TxnStatusReceived}}, false),
		txns("transactions/finality=CANDIDATE", &rpc.SubNewTxnsInput{FinalityStatus: []rpc.TxnStatus{rpc.TxnStatusCandidate}}, false),
		txns("transactions/finality=PRE_CONFIRMED", &rpc.SubNewTxnsInput{FinalityStatus: []rpc.TxnStatus{rpc.TxnStatusPreConfirmed}}, true),
		txns("transactions/finality=ACCEPTED_ON_L2", &rpc.SubNewTxnsInput{FinalityStatus: []rpc.TxnStatus{rpc.TxnStatusAcceptedOnL2}}, true),
		txns("transactions/finality=all", &rpc.SubNewTxnsInput{
			FinalityStatus: []rpc.TxnStatus{
				rpc.TxnStatusReceived,
				rpc.TxnStatusCandidate,
				rpc.TxnStatusPreConfirmed,
				rpc.TxnStatusAcceptedOnL2,
			},
		}, true),
		txns("transactions/sender=account0", &rpc.SubNewTxnsInput{SenderAddress: []*felt.Felt{account0}}, true),
		txns("transactions/sender=account1", &rpc.SubNewTxnsInput{SenderAddress: []*felt.Felt{account1}}, true),
		txns("transactions/sender=account0+PRE_CONFIRMED", &rpc.SubNewTxnsInput{
			FinalityStatus: []rpc.TxnStatus{rpc.TxnStatusPreConfirmed},
			SenderAddress:  []*felt.Felt{account0},
		}, true),
		txns("transactions/sender=account0+ACCEPTED_ON_L2", &rpc.SubNewTxnsInput{
			FinalityStatus: []rpc.TxnStatus{rpc.TxnStatusAcceptedOnL2},
			SenderAddress:  []*felt.Felt{account0},
		}, true),
		txns("transactions/tags=INCLUDE_PROOF_FACTS", &rpc.SubNewTxnsInput{
			Tags: []rpc.SubscriptionTag{rpc.SubTagIncludeProofFacts},
		}, true),
	}

	activeProbes := startProbes(t, probes, wsURL)
	defer closeProbes(activeProbes)
	writeFile(t, readyFile, "ready\n")

	hashes := readTxHashes(t, txFile)
	statusProbes := make([]probe, 0, len(hashes.Hashes))
	for i, hash := range hashes.Hashes {
		statusProbes = append(statusProbes, transactionStatus(fmt.Sprintf("transactionStatus/%d", i), mustFelt(t, hash), true))
	}
	activeStatusProbes := startProbes(t, statusProbes, wsURL)
	defer closeProbes(activeStatusProbes)
	writeFile(t, statusReadyFile, "ready\n")

	assertResults(t, awaitProbes(append(activeProbes, activeStatusProbes...), waitTimeout))
}

func startProbes(t *testing.T, probes []probe, wsURL string) []activeProbe {
	t.Helper()
	active := make([]activeProbe, 0, len(probes))
	for _, p := range probes {
		started, err := p.start(context.Background(), wsURL)
		if err != nil {
			closeProbes(active)
			t.Fatalf("%s subscribe: %v", p.name, err)
		}
		started.name = p.name
		started.required = p.required
		active = append(active, started)
	}
	return active
}

func awaitProbes(probes []activeProbe, wait time.Duration) []result {
	results := make([]result, len(probes))
	var wg sync.WaitGroup
	for i, p := range probes {
		wg.Add(1)
		go func() {
			defer wg.Done()
			results[i] = p.await(wait)
			results[i].name = p.name
		}()
	}
	wg.Wait()
	return results
}

func closeProbes(probes []activeProbe) {
	for _, p := range probes {
		if p.close != nil {
			p.close()
		}
	}
}

func assertResults(t *testing.T, results []result) {
	t.Helper()
	for _, r := range results {
		if r.err != nil {
			t.Errorf("%s failed: %v", r.name, r.err)
			continue
		}
		t.Logf("%-44s id=%-8s observed=%t %s", r.name, r.id, r.observed, r.detail)
	}
}

func newHeads(name string, blockID rpc.SubscriptionBlockID, required bool) probe {
	return probe{name: name, required: required, start: func(ctx context.Context, wsURL string) (activeProbe, error) {
		headers := make(chan *rpc.BlockHeader, 8)
		ws, err := rpc.NewWebsocketProvider(ctx, wsURL)
		if err != nil {
			return activeProbe{}, err
		}

		sub, err := ws.SubscribeNewHeads(ctx, headers, blockID)
		if err != nil {
			ws.Close()
			return activeProbe{}, err
		}

		return activeProbe{
			id:    sub.ID(),
			close: func() { sub.Unsubscribe(); ws.Close() },
			await: func(wait time.Duration) result {
				select {
				case h := <-headers:
					return result{id: sub.ID(), observed: true, detail: fmt.Sprintf("block=%d hash=%s", h.Number, h.Hash)}
				case err := <-sub.Err():
					return result{id: sub.ID(), err: err}
				case <-time.After(wait):
					return timeoutResult(sub.ID(), required)
				}
			},
		}, nil
	}}
}

func events(name string, input *rpc.EventSubscriptionInput, required bool) probe {
	return probe{name: name, required: required, start: func(ctx context.Context, wsURL string) (activeProbe, error) {
		events := make(chan *rpc.EmittedEventWithFinalityStatus, 8)
		ws, err := rpc.NewWebsocketProvider(ctx, wsURL)
		if err != nil {
			return activeProbe{}, err
		}

		sub, err := ws.SubscribeEvents(ctx, events, input)
		if err != nil {
			ws.Close()
			return activeProbe{}, err
		}

		return activeProbe{
			id:    sub.ID(),
			close: func() { sub.Unsubscribe(); ws.Close() },
			await: func(wait time.Duration) result {
				select {
				case e := <-events:
					return result{id: sub.ID(), observed: true, detail: fmt.Sprintf("tx=%s finality=%s block=%d", e.TransactionHash, e.FinalityStatus, e.BlockNumber)}
				case err := <-sub.Err():
					return result{id: sub.ID(), err: err}
				case <-time.After(wait):
					return timeoutResult(sub.ID(), required)
				}
			},
		}, nil
	}}
}

func receipts(name string, input *rpc.SubNewTxnReceiptsInput, required bool) probe {
	return probe{name: name, required: required, start: func(ctx context.Context, wsURL string) (activeProbe, error) {
		receipts := make(chan *rpc.TransactionReceiptWithBlockInfo, 8)
		ws, err := rpc.NewWebsocketProvider(ctx, wsURL)
		if err != nil {
			return activeProbe{}, err
		}

		sub, err := ws.SubscribeNewTransactionReceipts(ctx, receipts, input)
		if err != nil {
			ws.Close()
			return activeProbe{}, err
		}

		return activeProbe{
			id:    sub.ID(),
			close: func() { sub.Unsubscribe(); ws.Close() },
			await: func(wait time.Duration) result {
				select {
				case r := <-receipts:
					return result{id: sub.ID(), observed: true, detail: fmt.Sprintf("tx=%s finality=%s execution=%s block=%d", r.Hash, r.FinalityStatus, r.ExecutionStatus, r.BlockNumber)}
				case err := <-sub.Err():
					return result{id: sub.ID(), err: err}
				case <-time.After(wait):
					return timeoutResult(sub.ID(), required)
				}
			},
		}, nil
	}}
}

func txns(name string, input *rpc.SubNewTxnsInput, required bool) probe {
	return probe{name: name, required: required, start: func(ctx context.Context, wsURL string) (activeProbe, error) {
		txns := make(chan *rpc.TxnWithHashAndStatus, 8)
		ws, err := rpc.NewWebsocketProvider(ctx, wsURL)
		if err != nil {
			return activeProbe{}, err
		}

		sub, err := ws.SubscribeNewTransactions(ctx, txns, input)
		if err != nil {
			ws.Close()
			return activeProbe{}, err
		}

		return activeProbe{
			id:    sub.ID(),
			close: func() { sub.Unsubscribe(); ws.Close() },
			await: func(wait time.Duration) result {
				select {
				case tx := <-txns:
					return result{id: sub.ID(), observed: true, detail: fmt.Sprintf("tx=%s finality=%s", tx.Hash, tx.FinalityStatus)}
				case err := <-sub.Err():
					return result{id: sub.ID(), err: err}
				case <-time.After(wait):
					return timeoutResult(sub.ID(), required)
				}
			},
		}, nil
	}}
}

func transactionStatus(name string, hash *felt.Felt, required bool) probe {
	return probe{name: name, required: required, start: func(ctx context.Context, wsURL string) (activeProbe, error) {
		statuses := make(chan *rpc.NewTxnStatus, 8)
		ws, err := rpc.NewWebsocketProvider(ctx, wsURL)
		if err != nil {
			return activeProbe{}, err
		}

		sub, err := ws.SubscribeTransactionStatus(ctx, statuses, hash)
		if err != nil {
			ws.Close()
			return activeProbe{}, err
		}

		return activeProbe{
			id:    sub.ID(),
			close: func() { sub.Unsubscribe(); ws.Close() },
			await: func(wait time.Duration) result {
				select {
				case s := <-statuses:
					return result{id: sub.ID(), observed: true, detail: fmt.Sprintf("tx=%s finality=%s execution=%s", s.TransactionHash, s.Status.FinalityStatus, s.Status.ExecutionStatus)}
				case err := <-sub.Err():
					return result{id: sub.ID(), err: err}
				case <-time.After(wait):
					return timeoutResult(sub.ID(), required)
				}
			},
		}, nil
	}}
}

func timeoutResult(id string, required bool) result {
	if required {
		return result{id: id, err: fmt.Errorf("timed out waiting for notification")}
	}
	return result{id: id, detail: "subscription accepted; no matching notification during wait"}
}

func readTxHashes(t *testing.T, path string) txHashes {
	t.Helper()
	deadline := time.Now().Add(30 * time.Second)
	for time.Now().Before(deadline) {
		data, err := os.ReadFile(path)
		if err == nil {
			var hashes txHashes
			if err := json.Unmarshal(data, &hashes); err != nil {
				t.Fatalf("read tx hashes: %v", err)
			}
			if len(hashes.Hashes) == 0 {
				t.Fatalf("tx hash file is empty")
			}
			return hashes
		}
		if !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("read tx hash file: %v", err)
		}
		time.Sleep(100 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for tx hash file %s", path)
	return txHashes{}
}

func writeFile(t *testing.T, path string, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatalf("create coordination dir: %v", err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}

func mustEnv(t *testing.T, key string) string {
	t.Helper()
	value := os.Getenv(key)
	if value == "" {
		t.Fatalf("%s must be set", key)
	}
	return value
}

func mustFelt(t *testing.T, value string) *felt.Felt {
	t.Helper()
	f, err := new(felt.Felt).SetString(value)
	if err != nil {
		t.Fatalf("parse felt %q: %v", value, err)
	}
	return f
}
