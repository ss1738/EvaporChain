/**
 * WalletConnectScreen — WalletConnect v2 session management
 *
 * Paste a `wc:` URI to pair, see active sessions, and approve/reject
 * incoming proposals + signing requests. Wired to the singleton
 * `WalletConnectManager` from `state/wcContext.tsx` and routes
 * `evap_signTransaction` / `evap_signMessage` requests through the
 * on-device keystore (`utils/keystore.ts → signMessage`) which signs
 * with the real ML-DSA-65 secret key.
 *
 * UI parity with the extension's `WalletConnectScreen.tsx` /
 * `WcApprovalModal.tsx` — same flow, native primitives.
 */

import React, { useCallback, useEffect, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  Modal,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  TouchableOpacity,
  View,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { keystore } from '../utils/keystore';
import { useWalletConnect } from '../state/wcContext';
import type { WcRequest, WcSession, WcSessionProposal } from '../utils/walletconnect';

// ── Helpers ──

/** Coerce WC request params into a stable string we can sign. */
function extractSignablePayload(method: string, params: unknown): string {
  // Both `evap_signTransaction` and `evap_signMessage` accept either a
  // single string param or `[address, payload]` pairs (matching common
  // `personal_sign` conventions). Default to JSON-stringify for object
  // shapes so the user always sees something readable.
  if (Array.isArray(params)) {
    const first = params[0];
    if (typeof first === 'string') return first;
    return JSON.stringify(params);
  }
  if (typeof params === 'string') return params;
  return JSON.stringify(params);
}

/** Bytes the keystore actually signs for a given WC request. */
async function signForRequest(request: WcRequest): Promise<{ signature: string; message: string }> {
  const { method, params } = request.params.request;
  const payload = extractSignablePayload(method, params);
  // If the payload looks like 0x-hex bytes, sign the raw bytes;
  // otherwise sign the UTF-8 encoding of the string. This matches the
  // wallet/src/mnemonic.rs canonical-bytes convention used by the
  // extension's signTransaction surface.
  let bytes: Uint8Array;
  if (/^0x[0-9a-fA-F]*$/.test(payload) && payload.length % 2 === 0) {
    const hex = payload.slice(2);
    const out = new Uint8Array(hex.length / 2);
    for (let i = 0; i < out.length; i++) {
      out[i] = parseInt(hex.substr(i * 2, 2), 16);
    }
    bytes = out;
  } else {
    bytes = new TextEncoder().encode(payload);
  }
  const signature = await keystore.signMessage(bytes);
  return { signature, message: payload };
}

// ── Component ──

const WalletConnectScreen: React.FC = () => {
  const { manager, initialized, initError } = useWalletConnect();

  const [uri, setUri] = useState('');
  const [pairing, setPairing] = useState(false);
  const [sessions, setSessions] = useState<WcSession[]>([]);
  const [proposal, setProposal] = useState<WcSessionProposal | null>(null);
  const [pendingRequest, setPendingRequest] = useState<WcRequest | null>(null);
  const [requestBusy, setRequestBusy] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const refreshSessions = useCallback(() => {
    setSessions(manager.getActiveSessions());
  }, [manager]);

  // Wire SDK events + request handler when the manager is ready.
  useEffect(() => {
    if (!initialized) return;
    refreshSessions();

    manager.onProposal = (p) => {
      setPairing(false);
      setProposal(p);
    };
    manager.onDisconnect = () => {
      refreshSessions();
    };
    manager.onRequest = (req) => {
      // Surface the request as a pending modal; the user taps Approve
      // to actually invoke the handler. We do NOT auto-dispatch via
      // setRequestHandler so the user always sees the params first.
      setPendingRequest(req);
    };

    return () => {
      manager.onProposal = null;
      manager.onDisconnect = null;
      manager.onRequest = null;
    };
  }, [manager, initialized, refreshSessions]);

  // ── Pair ──

  const handlePair = useCallback(async () => {
    const trimmed = uri.trim();
    if (!trimmed) {
      setErrorMsg('Paste a WalletConnect URI (starts with wc:).');
      return;
    }
    setErrorMsg(null);
    setPairing(true);
    try {
      await manager.pair({ uri: trimmed });
      setUri('');
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setErrorMsg(msg);
      setPairing(false);
    }
  }, [manager, uri]);

  // ── Proposal approval ──

  const handleApproveProposal = useCallback(async () => {
    if (!proposal) return;
    try {
      const address = await keystore.getAddress();
      if (!address) {
        setErrorMsg('No wallet on device.');
        setProposal(null);
        return;
      }
      await manager.approveProposal(proposal, [address]);
      setProposal(null);
      refreshSessions();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setErrorMsg(msg);
    }
  }, [manager, proposal, refreshSessions]);

  const handleRejectProposal = useCallback(async () => {
    if (!proposal) return;
    try {
      await manager.rejectProposal(proposal);
    } catch {
      // Best-effort.
    }
    setProposal(null);
  }, [manager, proposal]);

  // ── Request approval ──

  const handleApproveRequest = useCallback(async () => {
    if (!pendingRequest) return;
    setRequestBusy(true);
    try {
      const { method } = pendingRequest.params.request;
      let result: unknown;
      switch (method) {
        case 'evap_getAccounts': {
          const addr = await keystore.getAddress();
          result = addr ? [addr] : [];
          break;
        }
        case 'evap_signTransaction':
        case 'evap_signMessage': {
          const { signature, message } = await signForRequest(pendingRequest);
          const publicKey = (await keystore.getPublicKey()) ?? '';
          // Response shape matches the extension's signTransaction
          // contract — the dApp gets the detached ML-DSA-65 signature
          // (hex), the public key (hex), and the canonical message
          // bytes back so it can verify locally.
          result = { signature, publicKey, message };
          break;
        }
        case 'evap_sendTransaction': {
          // The mobile wallet doesn't yet broadcast WC-routed
          // transactions itself — surface a clear error so the dApp
          // falls back to its own broadcaster.
          throw new Error('evap_sendTransaction not supported by this wallet build');
        }
        default:
          throw new Error(`Unsupported method: ${method}`);
      }
      await manager.respondSessionRequest(pendingRequest.topic, pendingRequest.id, result);
      setPendingRequest(null);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      try {
        await manager.rejectSessionRequest(pendingRequest.topic, pendingRequest.id, msg);
      } catch {
        // ignore
      }
      Alert.alert('Request Failed', msg);
      setPendingRequest(null);
    } finally {
      setRequestBusy(false);
    }
  }, [manager, pendingRequest]);

  const handleRejectRequest = useCallback(async () => {
    if (!pendingRequest) return;
    setRequestBusy(true);
    try {
      await manager.rejectSessionRequest(
        pendingRequest.topic,
        pendingRequest.id,
        'User rejected',
      );
    } catch {
      // ignore
    }
    setPendingRequest(null);
    setRequestBusy(false);
  }, [manager, pendingRequest]);

  // ── Disconnect ──

  const handleDisconnect = (topic: string, peerName: string) => {
    Alert.alert('Disconnect Session', `Disconnect from ${peerName}?`, [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Disconnect',
        style: 'destructive',
        onPress: async () => {
          try {
            await manager.disconnect(topic);
          } catch {
            // ignore
          }
          refreshSessions();
        },
      },
    ]);
  };

  // ── Render ──

  const formatTime = (ts: number) => {
    const d = new Date(ts);
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  };

  const proposalEvapNs = proposal
    ? proposal.params.requiredNamespaces?.evap ??
      Object.values(proposal.params.requiredNamespaces ?? {})[0] ??
      proposal.params.optionalNamespaces?.evap ??
      Object.values(proposal.params.optionalNamespaces ?? {})[0]
    : null;

  return (
    <SafeAreaView style={styles.safe} edges={['bottom']}>
      {/* SDK init banner */}
      {initError && (
        <View style={styles.errorBanner}>
          <Text style={styles.errorText}>{initError}</Text>
        </View>
      )}
      {!initialized && !initError && (
        <View style={styles.infoBanner}>
          <ActivityIndicator size="small" color="#7c3aed" />
          <Text style={styles.infoText}>Initializing WalletConnect…</Text>
        </View>
      )}

      {/* Pair input */}
      <View style={styles.pairBox}>
        <TextInput
          style={styles.uriInput}
          value={uri}
          onChangeText={setUri}
          placeholder="Paste WalletConnect URI (wc:…)"
          placeholderTextColor="#9ca3af"
          autoCapitalize="none"
          autoCorrect={false}
          editable={initialized}
        />
        <TouchableOpacity
          style={[
            styles.pairBtn,
            (!uri.trim() || pairing || !initialized) && styles.btnDisabled,
          ]}
          onPress={handlePair}
          disabled={!uri.trim() || pairing || !initialized}
        >
          {pairing ? (
            <ActivityIndicator color="#fff" size="small" />
          ) : (
            <Text style={styles.pairBtnText}>Connect</Text>
          )}
        </TouchableOpacity>
      </View>

      {errorMsg && (
        <View style={styles.errorBanner}>
          <Text style={styles.errorText}>{errorMsg}</Text>
        </View>
      )}

      {/* Sessions */}
      <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>
        <Text style={styles.sectionLabel}>
          Active Sessions {sessions.length > 0 ? `(${sessions.length})` : ''}
        </Text>
        {sessions.length === 0 ? (
          <View style={styles.emptyBlock}>
            <Text style={styles.emptyTitle}>No Active Sessions</Text>
            <Text style={styles.emptyText}>
              Paste a WalletConnect URI from a dApp to connect.
            </Text>
          </View>
        ) : (
          sessions.map((sess) => (
            <View key={sess.topic} style={styles.sessionCard}>
              <View style={styles.sessionHeader}>
                <View style={styles.sessionIconBox}>
                  <Text style={styles.sessionIcon}>WC</Text>
                </View>
                <View style={styles.sessionInfo}>
                  <Text style={styles.sessionName}>{sess.peer.name || 'Unknown dApp'}</Text>
                  <Text style={styles.sessionUrl} numberOfLines={1}>
                    {sess.peer.url}
                  </Text>
                  <Text style={styles.sessionConnected}>
                    Connected {formatTime(sess.connectedAt)}
                  </Text>
                </View>
                <TouchableOpacity onPress={() => handleDisconnect(sess.topic, sess.peer.name)}>
                  <Text style={styles.disconnectText}>Disconnect</Text>
                </TouchableOpacity>
              </View>
              <View style={styles.chainsRow}>
                {Object.values(sess.namespaces)
                  .flatMap((ns) => ns.chains)
                  .map((c) => (
                    <View key={c} style={styles.chainChip}>
                      <Text style={styles.chainChipText}>{c}</Text>
                    </View>
                  ))}
              </View>
            </View>
          ))
        )}
      </ScrollView>

      {/* Proposal approval modal */}
      <Modal
        visible={proposal !== null}
        transparent
        animationType="fade"
        onRequestClose={handleRejectProposal}
      >
        <View style={styles.modalBackdrop}>
          <View style={styles.modalCard}>
            <Text style={styles.modalTitle}>Connection Request</Text>
            {proposal && (
              <>
                <Text style={styles.modalPeerName}>
                  {proposal.params.proposer.metadata.name}
                </Text>
                <Text style={styles.modalPeerUrl}>
                  {proposal.params.proposer.metadata.url}
                </Text>
                {!!proposal.params.proposer.metadata.description && (
                  <Text style={styles.modalDesc}>
                    {proposal.params.proposer.metadata.description}
                  </Text>
                )}

                <Text style={styles.modalSectionLabel}>Requested Permissions</Text>
                <View style={styles.permList}>
                  {(proposalEvapNs?.methods ?? []).map((m) => (
                    <View key={m} style={styles.permRow}>
                      <Text style={styles.permText}>{methodLabel(m)}</Text>
                    </View>
                  ))}
                  {(proposalEvapNs?.methods ?? []).length === 0 && (
                    <Text style={styles.permEmpty}>
                      No EvaporChain permissions requested
                    </Text>
                  )}
                </View>

                <Text style={styles.modalSectionLabel}>Chains</Text>
                <View style={styles.chainsRow}>
                  {(proposalEvapNs?.chains ?? []).map((c) => (
                    <View key={c} style={styles.chainChip}>
                      <Text style={styles.chainChipText}>{c}</Text>
                    </View>
                  ))}
                </View>
              </>
            )}

            <View style={styles.modalActions}>
              <TouchableOpacity style={styles.rejectBtn} onPress={handleRejectProposal}>
                <Text style={styles.rejectBtnText}>Reject</Text>
              </TouchableOpacity>
              <TouchableOpacity style={styles.approveBtn} onPress={handleApproveProposal}>
                <Text style={styles.approveBtnText}>Approve</Text>
              </TouchableOpacity>
            </View>
          </View>
        </View>
      </Modal>

      {/* Request approval modal */}
      <Modal
        visible={pendingRequest !== null}
        transparent
        animationType="fade"
        onRequestClose={requestBusy ? undefined : handleRejectRequest}
      >
        <View style={styles.modalBackdrop}>
          <View style={styles.modalCard}>
            <Text style={styles.modalTitle}>dApp Request</Text>
            {pendingRequest && (
              <>
                <Text style={styles.modalPeerName}>
                  {pendingRequest.params.request.method}
                </Text>
                <Text style={styles.modalPeerUrl}>
                  {pendingRequest.params.chainId}
                </Text>
                <Text style={styles.modalSectionLabel}>Params</Text>
                <ScrollView style={styles.paramsBox}>
                  <Text style={styles.paramsText}>
                    {JSON.stringify(pendingRequest.params.request.params, null, 2)}
                  </Text>
                </ScrollView>
              </>
            )}

            <View style={styles.modalActions}>
              <TouchableOpacity
                style={[styles.rejectBtn, requestBusy && styles.btnDisabled]}
                onPress={handleRejectRequest}
                disabled={requestBusy}
              >
                <Text style={styles.rejectBtnText}>Reject</Text>
              </TouchableOpacity>
              <TouchableOpacity
                style={[styles.approveBtn, requestBusy && styles.btnDisabled]}
                onPress={handleApproveRequest}
                disabled={requestBusy}
              >
                {requestBusy ? (
                  <ActivityIndicator color="#fff" size="small" />
                ) : (
                  <Text style={styles.approveBtnText}>Approve</Text>
                )}
              </TouchableOpacity>
            </View>
          </View>
        </View>
      </Modal>
    </SafeAreaView>
  );
};

// ── Helpers ──

function methodLabel(m: string): string {
  switch (m) {
    case 'evap_getAccounts':
      return 'View accounts';
    case 'evap_signMessage':
      return 'Sign messages';
    case 'evap_signTransaction':
      return 'Sign transactions';
    case 'evap_sendTransaction':
      return 'Send transactions';
    default:
      return m;
  }
}

// ── Styles ──

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f9fafb' },
  pairBox: {
    flexDirection: 'row',
    gap: 8,
    padding: 12,
    backgroundColor: '#fff',
    borderBottomWidth: 1,
    borderBottomColor: '#e5e7eb',
  },
  uriInput: {
    flex: 1,
    height: 42,
    backgroundColor: '#f3f4f6',
    borderRadius: 10,
    borderWidth: 1,
    borderColor: '#e5e7eb',
    paddingHorizontal: 12,
    fontSize: 13,
    color: '#111827',
  },
  pairBtn: {
    backgroundColor: '#7c3aed',
    borderRadius: 10,
    paddingHorizontal: 16,
    justifyContent: 'center',
    alignItems: 'center',
  },
  pairBtnText: { color: '#fff', fontSize: 14, fontWeight: '600' },
  btnDisabled: { opacity: 0.4 },
  errorBanner: {
    backgroundColor: '#fef2f2',
    borderBottomWidth: 1,
    borderBottomColor: '#fecaca',
    paddingHorizontal: 14,
    paddingVertical: 10,
  },
  errorText: { fontSize: 13, color: '#b91c1c' },
  infoBanner: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 10,
    paddingHorizontal: 14,
    paddingVertical: 10,
    backgroundColor: '#f5f3ff',
    borderBottomWidth: 1,
    borderBottomColor: '#ddd6fe',
  },
  infoText: { fontSize: 13, color: '#5b21b6' },
  list: { flex: 1 },
  listContent: { padding: 12, gap: 10, paddingBottom: 32 },
  sectionLabel: {
    fontSize: 11,
    fontWeight: '700',
    color: '#6b7280',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginBottom: 4,
  },
  emptyBlock: { alignItems: 'center', paddingTop: 48, gap: 8 },
  emptyTitle: { fontSize: 15, fontWeight: '700', color: '#374151' },
  emptyText: { fontSize: 13, color: '#9ca3af', textAlign: 'center', maxWidth: 240 },
  sessionCard: {
    backgroundColor: '#fff',
    borderWidth: 1,
    borderColor: '#e5e7eb',
    borderRadius: 14,
    padding: 14,
    gap: 10,
  },
  sessionHeader: { flexDirection: 'row', alignItems: 'flex-start', gap: 10 },
  sessionIconBox: {
    width: 40,
    height: 40,
    borderRadius: 10,
    backgroundColor: '#ede9fe',
    alignItems: 'center',
    justifyContent: 'center',
  },
  sessionIcon: { fontSize: 12, fontWeight: '700', color: '#7c3aed' },
  sessionInfo: { flex: 1 },
  sessionName: { fontSize: 14, fontWeight: '700', color: '#111827' },
  sessionUrl: { fontSize: 11, color: '#9ca3af', marginTop: 1 },
  sessionConnected: { fontSize: 11, color: '#6b7280', marginTop: 2 },
  disconnectText: { fontSize: 12, color: '#ef4444', fontWeight: '600', paddingTop: 2 },
  chainsRow: { flexDirection: 'row', flexWrap: 'wrap', gap: 6 },
  chainChip: {
    backgroundColor: '#ede9fe',
    borderRadius: 6,
    paddingHorizontal: 8,
    paddingVertical: 3,
  },
  chainChipText: { fontSize: 10, color: '#7c3aed', fontWeight: '600' },
  // Modals
  modalBackdrop: {
    flex: 1,
    backgroundColor: 'rgba(0,0,0,0.5)',
    justifyContent: 'center',
    paddingHorizontal: 20,
  },
  modalCard: {
    backgroundColor: '#fff',
    borderRadius: 16,
    padding: 20,
    gap: 8,
  },
  modalTitle: {
    fontSize: 16,
    fontWeight: '700',
    color: '#111827',
    textAlign: 'center',
    marginBottom: 6,
  },
  modalPeerName: { fontSize: 15, fontWeight: '700', color: '#111827' },
  modalPeerUrl: { fontSize: 12, color: '#6b7280' },
  modalDesc: { fontSize: 13, color: '#374151', marginTop: 4 },
  modalSectionLabel: {
    fontSize: 10,
    fontWeight: '700',
    color: '#6b7280',
    textTransform: 'uppercase',
    letterSpacing: 0.5,
    marginTop: 10,
  },
  permList: { gap: 6, marginTop: 4 },
  permRow: {
    backgroundColor: '#f3f4f6',
    paddingHorizontal: 10,
    paddingVertical: 8,
    borderRadius: 8,
  },
  permText: { fontSize: 13, color: '#374151' },
  permEmpty: { fontSize: 12, color: '#9ca3af', fontStyle: 'italic' },
  paramsBox: {
    maxHeight: 160,
    backgroundColor: '#f9fafb',
    borderRadius: 8,
    padding: 10,
    marginTop: 4,
  },
  paramsText: { fontSize: 11, fontFamily: 'monospace', color: '#374151' },
  modalActions: { flexDirection: 'row', gap: 10, marginTop: 14 },
  rejectBtn: {
    flex: 1,
    paddingVertical: 12,
    borderRadius: 10,
    borderWidth: 1,
    borderColor: '#e5e7eb',
    alignItems: 'center',
  },
  rejectBtnText: { fontSize: 14, fontWeight: '600', color: '#6b7280' },
  approveBtn: {
    flex: 1,
    paddingVertical: 12,
    borderRadius: 10,
    backgroundColor: '#7c3aed',
    alignItems: 'center',
  },
  approveBtnText: { fontSize: 14, fontWeight: '600', color: '#fff' },
});

export default WalletConnectScreen;
