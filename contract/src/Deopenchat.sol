// SPDX-License-Identifier: GPL-3.0

pragma solidity >=0.7.0 <0.9.0;

interface IRiscZeroVerifier {
    /// @notice Verify that the given seal is a valid RISC Zero proof of execution with the
    ///     given image ID and journal digest. Reverts on failure.
    /// @dev This method additionally ensures that the input hash is all-zeros (i.e. no
    /// committed input), the exit code is (Halted, 0), and there are no assumptions (i.e. the
    /// receipt is unconditional).
    /// @param seal The encoded cryptographic proof (i.e. SNARK).
    /// @param imageId The identifier for the guest program.
    /// @param journalDigest The SHA-256 digest of the journal bytes.
    function verify(bytes calldata seal, bytes32 imageId, bytes32 journalDigest) external view;
}

contract Deopenchat {
    struct Record {
        uint32 seq;
        uint64 remainingTokens;
    }

    struct Provider {
        address providerAddress;
        uint32 costPerKTokens;
        string endpoint;
        string model;
    }

    address[] providers;
    // provider address -> provider
    mapping(address => Provider) providerMapping;
    // provider -> client -> record
    mapping(address => mapping(bytes32 => Record)) records;

    bytes32 imageId;
    address IRiscZeroContract;

    constructor(bytes32 id, address risc0Addr) {
        imageId = id;
        IRiscZeroContract = risc0Addr;
    }

    function providerRegister(
        uint32 ktokensCost,
        string calldata endpoint,
        string calldata model
    ) public {
        Provider memory p = Provider ({
            providerAddress: msg.sender,
            costPerKTokens: ktokensCost,
            endpoint: endpoint,
            model: model
        });

        providerMapping[msg.sender] = p;
        providers.push(msg.sender);
    }

    function getProvider(address provider) view public returns(Provider memory) {
        return providerMapping[provider];
    }

    function getImageId() view public returns(bytes32) {
        return imageId;
    }

    function getAllProviders() view public returns(Provider[] memory) {
        Provider[] memory ret = new Provider[](providers.length);

        for (uint32 i = 0; i < providers.length; i++) {
            ret[i] = providerMapping[providers[i]];
        }

        return ret;
    }

    function viewStatus(address provider, bytes32 clientPk) view public returns(Record memory)  {
        return records[provider][clientPk];
    }

    function fethTokens(address provider, uint32 ktokens, bytes32 clientPk) payable public {
        uint32 costPerKt = providerMapping[provider].costPerKTokens;
        require(costPerKt > 0);

        uint32 needCost = ktokens * costPerKt;
        require(needCost <= msg.value, "not enough amount!");

        payable(address(this)).transfer(msg.value);
        // todo payable(msg.sender).transfer()
        records[provider][clientPk].remainingTokens += ktokens * 1000;
    }

    struct Claim {
        bytes32 clientPk;
        uint32 seq;
        uint32 rounds;
        uint64 numberTokensConsumed;
    }

    // (clientPk + seq + rounds + numberTokensConsumed)
    uint constant CLAIM_SIZE = 32 + 4 + 4 + 8;

    function verifyTest(bytes calldata seal, bytes calldata journal) view public {
        IRiscZeroVerifier(IRiscZeroContract).verify(seal, imageId, sha256(journal));
    }

    function claim(Claim[] calldata claimList, bytes calldata seal) payable public {
        bytes memory journal = new bytes(CLAIM_SIZE * claimList.length);
        uint64 totalTokensUsage = 0;

        for (uint32 i = 0; i < claimList.length; i++) {
            Claim calldata c = claimList[i];
            bytes32 clientPk = c.clientPk;
            bytes4 seq = bytes4(c.seq);
            bytes4 rounds = bytes4(c.rounds);
            bytes8 numberTokensConsumed = bytes8(c.numberTokensConsumed);

            uint pkoffset = 32 + CLAIM_SIZE * i;
            uint seqoffset = pkoffset + 32;
            uint roundsoffset = seqoffset + 4;
            uint numberTokensConsumedoffset = roundsoffset + 4;

            assembly {
                mstore(add(journal, pkoffset), clientPk)
                mstore(add(journal, seqoffset), seq)
                mstore(add(journal, roundsoffset), rounds)
                mstore(add(journal, numberTokensConsumedoffset), numberTokensConsumed)
            }

            require(records[msg.sender][c.clientPk].remainingTokens >= c.numberTokensConsumed, "no enough tokens");
            require(records[msg.sender][c.clientPk].seq + 1 == c.seq);

            records[msg.sender][c.clientPk].remainingTokens -= c.numberTokensConsumed;
            records[msg.sender][c.clientPk].seq += c.rounds;
            totalTokensUsage += c.numberTokensConsumed;
        }

        IRiscZeroVerifier(IRiscZeroContract).verify(seal, imageId, sha256(journal));
        payable(msg.sender).transfer(totalTokensUsage / 1000 * providerMapping[msg.sender].costPerKTokens);
    }

    fallback() external payable {}

    receive() external payable {}
}