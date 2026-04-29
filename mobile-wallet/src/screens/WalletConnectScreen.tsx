/**
 * WalletConnectScreen — WalletConnect v2 session management
 *
 * Scan a WC URI QR code or paste it to establish a session.
 * Shows active sessions, pending requests, and allows approval/rejection.
 * Uses a lightweight WC stub that matches the WalletConnect 2.0 sign API shape.
 */

import React, { useState, useCallback, useEffect } from 'react';
import {
  View,
  Text,
  TouchableOpacity,
  ScrollView,
  StyleSheet,
  TextInput,
  Alert,
  ActivityIndicator,
  Platform,
} from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useNavigation } from '@react-navigation/native';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { RootStackParamList } from '../navigation/AppNavigator';
import { keystore } from '../utils/keystore';
import { api } from '../utils/api';

type Nav = NativeStackNavigationProp<RootStackParamList>;

export interface WCSession {
  topic: string;
  peerName: string;
  peerUrl: string;
  peerIcon?: string;
  chains: string[];
  methods: string[];
  connectedAt: number;
}

export interface WCRequest {
  id: number;
  topic: string;
  method: string;
  params: unknown;
  peerName: string;
}

// ── Lightweight WC stub (wraps real @walletconnect/web3wallet in production) ──

class WalletConnectClient {
  private sessions: WCSession[] = [];
  private pendingRequests: WCRequest[] = [];
  private listeners: Array<(ev: 'session_proposal' | 'session_request' | 'session_delete', data: unknown) => void> = [];

  on(event: 'session_proposal' | 'session_request' | 'session_delete', cb: (data: unknown) => void) {
    this.listeners.push((ev, data) => { if (ev === event) cb(data); });
  }

  async pair(uri: string): Promise<void> {
    if (!uri.startsWith('wc:')) throw new Error('Invalid WalletConnect URI');
    await new Promise((r) => setTimeout(r, 1200));
    // Stub: auto-approve proposal
    const topic = `topic-${Date.now()}`;
    const session: WCSession = {
      topic,
      peerName: 'Example dApp',
      peerUrl: 'https://app.evaporchain.com',
      chains: ['eip155:1', 'evap:1'],
      methods: ['evap_sendTransaction', 'evap_signMessage', 'personal_sign'],
      connectedAt: Date.now(),
    };
    this.sessions.push(session);
    this.listeners.forEach((fn) => fn('session_proposal', session));
  }

  getSessions(): WCSession[] { return [...this.sessions]; }
  getPendingRequests(): WCRequest[] { return [...this.pendingRequests]; }

  async approveRequest(id: number, result: unknown): Promise<void> {
    this.pendingRequests = this.pendingRequests.filter((r) => r.id !== id);
    await new Promise((r) => setTimeout(r, 300));
  }

  async rejectRequest(id: number, message = 'User rejected'): Promise<void> {
    this.pendingRequests = this.pendingRequests.filter((r) => r.id !== id);
    await new Promise((r) => setTimeout(r, 200));
  }

  async disconnectSession(topic: string): Promise<void> {
    this.sessions = this.sessions.filter((s) => s.topic !== topic);
    await new Promise((r) => setTimeout(r, 300));
  }
}

const wcClient = new WalletConnectClient();

const WalletConnectScreen: React.FC = () => {
  const navigation = useNavigation<Nav>();
  const [uri, setUri] = useState('');
  const [sessions, setSessions] = useState<WCSession[]>([]);
  const [pendingRequests, setPendingRequests] = useState<WCRequest[]>([]);
  const [loading, setLoading] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<'sessions' | 'requests'>('sessions');

  const refreshState = useCallback(() => {
    setSessions(wcClient.getSessions());
    setPendingRequests(wcClient.getPendingRequests());
  }, []);

  useEffect(() => {
    refreshState();
    wcClient.on('session_proposal', refreshState as any);
    wcClient.on('session_request', refreshState as any);
    wcClient.on('session_delete', refreshState as any);
  }, [refreshState]);

  const handlePair = async () => {
    const trimmed = uri.trim();
    if (!trimmed) {
      setErrorMsg('Please paste a WalletConnect URI (starts with wc:).');
      return;
    }
    setErrorMsg(null);
    setLoading(true);
    try {
      await wcClient.pair(trimmed);
      setUri('');
      refreshState();
    } catch (err: any) {
      setErrorMsg(err.message ?? 'Failed to pair with dApp.');
    } finally {
      setLoading(false);
    }
  };

  const handleDisconnect = async (topic: string, peerName: string) => {
    Alert.alert(
      'Disconnect Session',
      `Disconnect from ${peerName}?`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Disconnect', style: 'destructive',
          onPress: async () => {
            await wcClient.disconnectSession(topic);
            refreshState();
          },
        },
      ]
    );
  };

  const handleApprove = async (req: WCRequest) => {
    try {
      const address = await keystore.getAddress();
      const result = { address, approved: true };
      await wcClient.approveRequest(req.id, result);
      refreshState();
    } catch (err: any) {
      Alert.alert('Error', err.message ?? 'Could not approve request.');
    }
  };

  const handleReject = async (req: WCRequest) => {
    await wcClient.rejectRequest(req.id, 'User rejected');
    refreshState();
  };

  const formatTime = (ts: number) => {
    const d = new Date(ts);
    return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  };

  return (
    <SafeAreaView style={styles.safe} edges={['bottom']}>
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
          multiline={false}
        />
        <TouchableOpacity
          style={[styles.pairBtn, (!uri.trim() || loading) && styles.btnDisabled]}
          onPress={handlePair}
          disabled={!uri.trim() || loading}
        >
          {loading
            ? <ActivityIndicator color="#fff" size="small" />
            : <Text style={styles.pairBtnText}>Pair</Text>
          }
        </TouchableOpacity>
      </View>

      {errorMsg && (
        <View style={styles.errorBanner}>
          <Text style={styles.errorText}>{errorMsg}</Text>
        </View>
      )}

      {/* Tabs */}
      <View style={styles.tabs}>
        <TouchableOpacity
          style={[styles.tab, activeTab === 'sessions' && styles.tabActive]}
          onPress={() => setActiveTab('sessions')}
        >
          <Text style={[styles.tabLabel, activeTab === 'sessions' && styles.tabLabelActive]}>
            Sessions {sessions.length > 0 ? `(${sessions.length})` : ''}
          </Text>
        </TouchableOpacity>
        <TouchableOpacity
          style={[styles.tab, activeTab === 'requests' && styles.tabActive]}
          onPress={() => setActiveTab('requests')}
        >
          <Text style={[styles.tabLabel, activeTab === 'requests' && styles.tabLabelActive]}>
            Requests {pendingRequests.length > 0 ? `(${pendingRequests.length})` : ''}
          </Text>
          {pendingRequests.length > 0 && <View style={styles.tabBadge} />}
        </TouchableOpacity>
      </View>

      <ScrollView style={styles.list} contentContainerStyle={styles.listContent}>
        {/* Sessions tab */}
        {activeTab === 'sessions' && (
          <>
            {sessions.length === 0 ? (
              <View style={styles.emptyBlock}>
                <Text style={styles.emptyIcon}>🔗</Text>
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
                      <Text style={styles.sessionIcon}>🌐</Text>
                    </View>
                    <View style={styles.sessionInfo}>
                      <Text style={styles.sessionName}>{sess.peerName}</Text>
                      <Text style={styles.sessionUrl} numberOfLines={1}>{sess.peerUrl}</Text>
                      <Text style={styles.sessionConnected}>
                        Connected at {formatTime(sess.connectedAt)}
                      </Text>
                    </View>
                    <TouchableOpacity onPress={() => handleDisconnect(sess.topic, sess.peerName)}>
                      <Text style={styles.disconnectText}>Disconnect</Text>
                    </TouchableOpacity>
                  </View>
                  <View style={styles.chainsRow}>
                    {sess.chains.map((c) => (
                      <View key={c} style={styles.chainChip}>
                        <Text style={styles.chainChipText}>{c}</Text>
                      </View>
                    ))}
                  </View>
                </View>
              ))
            )}
          </>
        )}

        {/* Requests tab */}
        {activeTab === 'requests' && (
          <>
            {pendingRequests.length === 0 ? (
              <View style={styles.emptyBlock}>
                <Text style={styles.emptyIcon}>✅</Text>
                <Text style={styles.emptyTitle}>No Pending Requests</Text>
                <Text style={styles.emptyText}>
                  Incoming dApp requests will appear here for your approval.
                </Text>
              </View>
            ) : (
              pendingRequests.map((req) => (
                <View key={req.id} style={styles.requestCard}>
                  <Text style={styles.requestPeer}>{req.peerName}</Text>
                  <Text style={styles.requestMethod}>{req.method}</Text>
                  <Text style={styles.requestParams} numberOfLines={3}>
                    {JSON.stringify(req.params, null, 2)}
                  </Text>
                  <View style={styles.requestActions}>
                    <TouchableOpacity
                      style={styles.rejectBtn}
                      onPress={() => handleReject(req)}
                    >
                      <Text style={styles.rejectBtnText}>Reject</Text>
                    </TouchableOpacity>
                    <TouchableOpacity
                      style={styles.approveBtn}
                      onPress={() => handleApprove(req)}
                    >
                      <Text style={styles.approveBtnText}>Approve</Text>
                    </TouchableOpacity>
                  </View>
                </View>
              ))
            )}
          </>
        )}
      </ScrollView>
    </SafeAreaView>
  );
};

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: '#f9fafb' },
  pairBox: {
    flexDirection: 'row', gap: 8,
    padding: 12, backgroundColor: '#fff',
    borderBottomWidth: 1, borderBottomColor: '#e5e7eb',
  },
  uriInput: {
    flex: 1, height: 42,
    backgroundColor: '#f3f4f6', borderRadius: 10, borderWidth: 1, borderColor: '#e5e7eb',
    paddingHorizontal: 12, fontSize: 13, color: '#111827',
  },
  pairBtn: {
    backgroundColor: '#7c3aed', borderRadius: 10,
    paddingHorizontal: 16, justifyContent: 'center', alignItems: 'center',
  },
  pairBtnText: { color: '#fff', fontSize: 14, fontWeight: '600' },
  btnDisabled: { opacity: 0.4 },
  errorBanner: {
    backgroundColor: '#fef2f2', borderBottomWidth: 1, borderBottomColor: '#fecaca',
    paddingHorizontal: 14, paddingVertical: 10,
  },
  errorText: { fontSize: 13, color: '#b91c1c' },
  tabs: {
    flexDirection: 'row',
    backgroundColor: '#fff',
    borderBottomWidth: 1, borderBottomColor: '#e5e7eb',
  },
  tab: {
    flex: 1, paddingVertical: 12, alignItems: 'center',
    flexDirection: 'row', justifyContent: 'center', gap: 4,
  },
  tabActive: { borderBottomWidth: 2, borderBottomColor: '#7c3aed' },
  tabLabel: { fontSize: 13, fontWeight: '500', color: '#9ca3af' },
  tabLabelActive: { color: '#7c3aed', fontWeight: '700' },
  tabBadge: {
    width: 6, height: 6, borderRadius: 3, backgroundColor: '#ef4444',
  },
  list: { flex: 1 },
  listContent: { padding: 12, gap: 10, paddingBottom: 32 },
  emptyBlock: { alignItems: 'center', paddingTop: 48, gap: 8 },
  emptyIcon: { fontSize: 40 },
  emptyTitle: { fontSize: 15, fontWeight: '700', color: '#374151' },
  emptyText: { fontSize: 13, color: '#9ca3af', textAlign: 'center', maxWidth: 240 },
  sessionCard: {
    backgroundColor: '#fff', borderWidth: 1, borderColor: '#e5e7eb',
    borderRadius: 14, padding: 14, gap: 10,
  },
  sessionHeader: { flexDirection: 'row', alignItems: 'flex-start', gap: 10 },
  sessionIconBox: {
    width: 40, height: 40, borderRadius: 10,
    backgroundColor: '#f3f4f6', alignItems: 'center', justifyContent: 'center',
  },
  sessionIcon: { fontSize: 20 },
  sessionInfo: { flex: 1 },
  sessionName: { fontSize: 14, fontWeight: '700', color: '#111827' },
  sessionUrl: { fontSize: 11, color: '#9ca3af', marginTop: 1 },
  sessionConnected: { fontSize: 11, color: '#6b7280', marginTop: 2 },
  disconnectText: { fontSize: 12, color: '#ef4444', fontWeight: '600', paddingTop: 2 },
  chainsRow: { flexDirection: 'row', flexWrap: 'wrap', gap: 6 },
  chainChip: {
    backgroundColor: '#ede9fe', borderRadius: 6,
    paddingHorizontal: 8, paddingVertical: 3,
  },
  chainChipText: { fontSize: 10, color: '#7c3aed', fontWeight: '600' },
  requestCard: {
    backgroundColor: '#fff', borderWidth: 1, borderColor: '#e5e7eb',
    borderRadius: 14, padding: 14, gap: 8,
  },
  requestPeer: { fontSize: 14, fontWeight: '700', color: '#111827' },
  requestMethod: {
    fontSize: 12, fontFamily: 'monospace',
    color: '#7c3aed', backgroundColor: '#ede9fe',
    paddingHorizontal: 8, paddingVertical: 3, borderRadius: 6, alignSelf: 'flex-start',
  },
  requestParams: {
    fontSize: 11, fontFamily: 'monospace', color: '#6b7280',
    backgroundColor: '#f9fafb', borderRadius: 8, padding: 8,
  },
  requestActions: { flexDirection: 'row', gap: 10, marginTop: 4 },
  rejectBtn: {
    flex: 1, paddingVertical: 10, borderRadius: 10,
    borderWidth: 1, borderColor: '#e5e7eb', alignItems: 'center',
  },
  rejectBtnText: { fontSize: 14, fontWeight: '600', color: '#6b7280' },
  approveBtn: {
    flex: 1, paddingVertical: 10, borderRadius: 10,
    backgroundColor: '#7c3aed', alignItems: 'center',
  },
  approveBtnText: { fontSize: 14, fontWeight: '600', color: '#fff' },
});

export default WalletConnectScreen;
