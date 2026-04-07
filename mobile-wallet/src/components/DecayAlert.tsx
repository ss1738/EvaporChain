/**
 * DecayAlert — Banner showing the most urgent decaying object/NFT.
 *
 * Displayed when any owned asset drops below 20% energy.
 * Tappable to navigate to the Objects or NFTs screen.
 */

import React from 'react';
import { View, Text, TouchableOpacity, StyleSheet } from 'react-native';
import type { ChainObject, NFT } from '../utils/api';

interface DecayAlertProps {
  objects: ChainObject[];
  nfts: NFT[];
  onPressObject?: (objectId: string) => void;
  onPressNft?: (nftId: string) => void;
}

interface UrgentItem {
  id: string;
  name: string;
  percentage: number;
  hoursLeft: number;
  type: 'object' | 'nft';
}

export const DecayAlert: React.FC<DecayAlertProps> = ({
  objects,
  nfts,
  onPressObject,
  onPressNft,
}) => {
  const urgentItems: UrgentItem[] = [];

  for (const obj of objects) {
    const pct = obj.maxEnergy > 0 ? obj.energy / obj.maxEnergy : 0;
    if (pct < 0.2 && obj.state !== 'Ghost') {
      urgentItems.push({
        id: obj.id,
        name: obj.name,
        percentage: Math.round(pct * 100),
        hoursLeft: Math.max(0, (obj.estimatedGhostTime - Date.now()) / 3600000),
        type: 'object',
      });
    }
  }

  for (const nft of nfts) {
    const pct = nft.maxEnergy > 0 ? nft.energy / nft.maxEnergy : 0;
    if (pct < 0.2 && nft.state !== 'Ghost') {
      urgentItems.push({
        id: nft.id,
        name: nft.name,
        percentage: Math.round(pct * 100),
        hoursLeft: Math.max(0, (nft.estimatedGhostTime - Date.now()) / 3600000),
        type: 'nft',
      });
    }
  }

  if (urgentItems.length === 0) return null;

  // Sort by hours left ascending (most urgent first)
  urgentItems.sort((a, b) => a.hoursLeft - b.hoursLeft);
  const most = urgentItems[0];

  const hoursText =
    most.hoursLeft < 1
      ? `${Math.round(most.hoursLeft * 60)}m`
      : `${Math.round(most.hoursLeft)}h`;

  const handlePress = () => {
    if (most.type === 'object' && onPressObject) {
      onPressObject(most.id);
    } else if (most.type === 'nft' && onPressNft) {
      onPressNft(most.id);
    }
  };

  return (
    <TouchableOpacity
      style={styles.container}
      onPress={handlePress}
      activeOpacity={0.8}
    >
      <View style={styles.iconContainer}>
        <Text style={styles.icon}>!</Text>
      </View>
      <View style={styles.content}>
        <Text style={styles.title}>Decay Alert</Text>
        <Text style={styles.message}>
          "{most.name}" at {most.percentage}% energy — {hoursText} until Ghost
        </Text>
        {urgentItems.length > 1 && (
          <Text style={styles.more}>
            +{urgentItems.length - 1} more asset{urgentItems.length > 2 ? 's' : ''} at risk
          </Text>
        )}
      </View>
      <Text style={styles.arrow}>{'>'}</Text>
    </TouchableOpacity>
  );
};

const styles = StyleSheet.create({
  container: {
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: '#fef3c7',
    borderWidth: 1,
    borderColor: '#f59e0b',
    borderRadius: 12,
    padding: 12,
    marginHorizontal: 16,
    marginVertical: 8,
  },
  iconContainer: {
    width: 32,
    height: 32,
    borderRadius: 16,
    backgroundColor: '#f59e0b',
    alignItems: 'center',
    justifyContent: 'center',
    marginRight: 12,
  },
  icon: {
    color: '#ffffff',
    fontSize: 18,
    fontWeight: '800',
  },
  content: {
    flex: 1,
  },
  title: {
    fontSize: 13,
    fontWeight: '700',
    color: '#92400e',
    marginBottom: 2,
  },
  message: {
    fontSize: 12,
    color: '#78350f',
    lineHeight: 16,
  },
  more: {
    fontSize: 11,
    color: '#a16207',
    marginTop: 2,
  },
  arrow: {
    fontSize: 16,
    color: '#a16207',
    fontWeight: '600',
    marginLeft: 8,
  },
});

export default DecayAlert;
