// SPDX-License-Identifier: GPL-3.0
pragma solidity >=0.7.0 <0.9.0;

contract DummyContract {
    bool isCanceled;
    // State update fields
    uint256 _globalRoot = 0;
    int256 _blockNumber = -1;
    uint256 _blockHash = 0;

    event LogMessageToL2(address indexed _fromAddress, uint256 indexed _toAddress, uint256 indexed _selector, uint256[] payload, uint256 nonce, uint256 fee);
    event LogStateUpdate(uint256 globalRoot, int256 blockNumber, uint256 blockHash);

    struct MessageData {
        address fromAddress;
        uint256 toAddress;
        uint256 selector;
        uint256[] payload;
        uint256 nonce;
        uint256 fee;
    }

    function getMessageData() internal pure returns (MessageData memory) {
        address fromAddress = address(993696174272377493693496825928908586134624850969);
        uint256 toAddress = 3256441166037631918262930812410838598500200462657642943867372734773841898370;
        uint256 selector = 774397379524139446221206168840917193112228400237242521560346153613428128537;
        uint256[] memory payload = new uint256[](7);
        payload[0] = 96;
        payload[1] = 1659025;
        payload[2] = 38575600093162;
        payload[3] = 5;
        payload[4] = 4543560;
        payload[5] = 1082959358903034162641917759097118582889062097851;
        payload[6] = 221696535382753200248526706088340988821219073423817576256483558730535647368;
        uint256 nonce = 0;
        uint256 fee = 0;
        return MessageData(fromAddress, toAddress, selector, payload, nonce, fee);
    }

    function fireEvent() public {
        MessageData memory data = getMessageData();
        emit LogMessageToL2(data.fromAddress, data.toAddress, data.selector, data.payload, data.nonce, data.fee);
    }

    function l1ToL2MessageCancellations(bytes32) external view returns (uint256) {
        return isCanceled ? 1723134213 : 0;
    }

    function l1ToL2Messages(bytes32) external view returns (uint256) {
        MessageData memory data = getMessageData();
        return data.fee + 1;
    }

    function setIsCanceled(bool value) public {
        isCanceled = value;
    }

    function getL1ToL2MsgHash() external pure returns (bytes32) {
        MessageData memory data = getMessageData();
        return keccak256(
            abi.encodePacked(
                uint256(uint160(data.fromAddress)),
                data.toAddress,
                data.nonce,
                data.selector,
                data.payload.length,
                data.payload
            )
        );
    }

    // State update view functions (return dummy values)
    function stateBlockNumber() public view returns (int256) {
        return _blockNumber;
    }

    function stateRoot() public view returns (uint256) {
        return _globalRoot;
    }

    function stateBlockHash() public view returns (uint256) {
        return _blockHash;
    }

    function programHash() public pure returns (uint256) {
        return 0;
    }

    function configHash() public pure returns (uint256) {
        return 0;
    }
}
