// SuperBizAgent 前端应用
class SuperBizAgentApp {
    constructor() {
        // 动态API基础URL - 自动适配当前环境
        this.apiBaseUrl = this.detectApiBaseUrl();
        this.currentMode = 'quick'; // 'quick' 或 'stream'
        this.sessionId = this.generateSessionId();
        this.isStreaming = false;
        this.currentChatHistory = []; // 当前对话的消息历史
        this.chatHistories = this.loadChatHistories(); // 所有历史对话
        this.isCurrentChatFromHistory = false; // 标记当前对话是否是从历史记录加载的
        this.alertEventSource = null; // SSE连接实例
        this.pendingAlertId = null; // 待查看的告警ID
        this.incidentDetailRefreshTimer = null;
        this.incidentDetailRenderSignature = null;
        this.dependencies = [];
        this.dependencyLoadError = '';

        this.initializeElements();
        this.initTheme();
        this.bindEvents();
        this.updateUI();
        this.initMarkdown();
        this.checkAndSetCentered();
        this.renderChatHistory();
        this.loadServerChatHistories();
        this.connectAlertSSE();
        this.setupMobileSidebar();
    }

    // 动态检测API基础URL
    detectApiBaseUrl() {
        const configured = this.readConfiguredApiBaseUrl();
        if (configured) {
            return configured;
        }

        if (window.location.protocol === 'http:' || window.location.protocol === 'https:') {
            return `${window.location.origin}/api`;
        }

        return 'http://localhost:3000/api';
    }

    readConfiguredApiBaseUrl() {
        const normalize = (value) => {
            const trimmed = String(value || '').trim();
            return trimmed ? trimmed.replace(/\/+$/, '') : '';
        };

        try {
            const params = new URLSearchParams(window.location.search);
            const queryValue = normalize(params.get('apiBaseUrl'));
            if (queryValue) return queryValue;
        } catch (e) {
            // Ignore malformed or unavailable location data.
        }

        const globalValue = normalize(window.ONCALL_API_BASE_URL);
        if (globalValue) return globalValue;

        try {
            return normalize(localStorage.getItem('apiBaseUrl'));
        } catch (e) {
            return '';
        }
    }

    // ==================== Markdown ====================

    // 初始化Markdown配置 (marked v11+使用非标准highlight方式)
    initMarkdown() {
        const checkMarked = () => {
            if (typeof marked !== 'undefined') {
                try {
                    marked.setOptions({
                        breaks: true,
                        gfm: true,
                        headerIds: false,
                        mangle: false
                    });
                    if (typeof hljs !== 'undefined') {
                        // marked v11+ 不再支持 highlight 选项，使用扩展或手动高亮
                        console.log('Markdown 和代码高亮库初始化成功');
                    }
                } catch (e) {
                    console.error('Markdown 配置失败:', e);
                }
            } else {
                setTimeout(checkMarked, 100);
            }
        };
        checkMarked();
    }

    // 安全地渲染 Markdown
    renderMarkdown(content) {
        if (!content) return '';
        if (typeof marked === 'undefined') {
            return this.escapeHtml(content);
        }
        try {
            const html = marked.parse(content);
            if (typeof DOMPurify !== 'undefined') {
                return DOMPurify.sanitize(html);
            }
            console.warn('DOMPurify 未加载，Markdown 内容将以纯文本显示');
            return this.escapeHtml(content);
        } catch (e) {
            console.error('Markdown 渲染失败:', e);
            return this.escapeHtml(content);
        }
    }

    // 高亮代码块
    highlightCodeBlocks(container) {
        if (typeof hljs !== 'undefined' && container) {
            try {
                container.querySelectorAll('pre code:not(.hljs)').forEach((block) => {
                    hljs.highlightElement(block);
                });
            } catch (e) {
                // 静默处理高亮失败
            }
        }
    }

    // ==================== DOM初始化 ====================

    initializeElements() {
        // 侧边栏元素
        this.sidebar = document.querySelector('.sidebar');
        this.newChatBtn = document.getElementById('newChatBtn');
        this.sidebarBackdrop = document.getElementById('sidebarBackdrop');
        this.themeToggleBtn = document.getElementById('themeToggleBtn');
        this.themeIconSun = document.getElementById('themeIconSun');
        this.themeIconMoon = document.getElementById('themeIconMoon');

        // 输入区域元素
        this.messageInput = document.getElementById('messageInput');
        this.sendButton = document.getElementById('sendButton');
        this.toolsBtn = document.getElementById('toolsBtn');
        this.toolsMenu = document.getElementById('toolsMenu');
        this.uploadFileItem = document.getElementById('uploadFileItem');
        this.modeSelectorBtn = document.getElementById('modeSelectorBtn');
        this.modeDropdown = document.getElementById('modeDropdown');
        this.currentModeText = document.getElementById('currentModeText');
        this.fileInput = document.getElementById('fileInput');

        // 聊天区域元素
        this.chatMessages = document.getElementById('chatMessages');
        this.loadingOverlay = document.getElementById('loadingOverlay');
        this.loadingText = document.getElementById('loadingText');
        this.loadingSubtext = document.getElementById('loadingSubtext');
        this.chatContainer = document.querySelector('.chat-container');
        this.welcomeGreeting = document.getElementById('welcomeGreeting');
        this.chatHistoryList = document.getElementById('chatHistoryList');

        // 告警相关元素
        this.queryAlertsBtn = document.getElementById('queryAlertsBtn');
        this.alertHistoryBtn = document.getElementById('alertHistoryBtn');
        this.simulateAlertBtn = document.getElementById('simulateAlertBtn');
        this.alertHistoryPanel = document.getElementById('alertHistoryPanel');
        this.alertHistoryContent = document.getElementById('alertHistoryContent');
        this.alertHistoryPanelClose = document.getElementById('alertHistoryPanelClose');
        this.alertDetailPanel = document.getElementById('alertDetailPanel');
        this.alertDetailContent = document.getElementById('alertDetailContent');
        this.alertDetailPanelClose = document.getElementById('alertDetailPanelClose');

        // 知识库相关元素
        this.knowledgePanelBtn = document.getElementById('knowledgePanelBtn');
        this.knowledgePanel = document.getElementById('knowledgePanel');
        this.knowledgePanelContent = document.getElementById('knowledgePanelContent');
        this.knowledgePanelClose = document.getElementById('knowledgePanelClose');
    }

    // ==================== 事件绑定 ====================

    bindEvents() {
        // 新建对话
        if (this.newChatBtn) {
            this.newChatBtn.addEventListener('click', () => this.newChat());
        }

        // 模式选择下拉菜单
        if (this.modeSelectorBtn) {
            this.modeSelectorBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                this.toggleModeDropdown();
            });
        }

        // 下拉菜单项点击
        document.querySelectorAll('.dropdown-item').forEach(item => {
            item.addEventListener('click', () => {
                const mode = item.getAttribute('data-mode');
                if (mode) {
                    this.selectMode(mode);
                    this.closeModeDropdown();
                }
            });
        });

        // 点击外部关闭下拉菜单
        document.addEventListener('click', (e) => {
            if (this.modeSelectorBtn && this.modeDropdown &&
                !this.modeSelectorBtn.contains(e.target) &&
                !this.modeDropdown.contains(e.target)) {
                this.closeModeDropdown();
            }
        });

        // 发送消息
        if (this.sendButton) {
            this.sendButton.addEventListener('click', () => this.sendMessage());
        }

        if (this.messageInput) {
            this.messageInput.addEventListener('keydown', (e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                    e.preventDefault();
                    this.sendMessage();
                }
            });
            // 输入框自动调整高度
            this.messageInput.addEventListener('input', () => this.autoResizeInput());
        }

        // 工具按钮和菜单
        if (this.toolsBtn) {
            this.toolsBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                this.toggleToolsMenu();
            });
        }

        if (this.uploadFileItem) {
            this.uploadFileItem.addEventListener('click', () => {
                if (this.fileInput) this.fileInput.click();
                this.closeToolsMenu();
            });
        }

        // 点击外部关闭工具菜单
        document.addEventListener('click', (e) => {
            if (this.toolsBtn && this.toolsMenu &&
                !this.toolsBtn.contains(e.target) &&
                !this.toolsMenu.contains(e.target)) {
                this.closeToolsMenu();
            }
        });

        if (this.fileInput) {
            this.fileInput.addEventListener('change', (e) => this.handleFileSelect(e));
        }

        // 告警相关事件绑定
        if (this.queryAlertsBtn) {
            this.queryAlertsBtn.addEventListener('click', () => this.triggerAIOps());
        }
        if (this.alertHistoryBtn) {
            this.alertHistoryBtn.addEventListener('click', () => this.showAlertHistory());
        }
        if (this.simulateAlertBtn) {
            this.simulateAlertBtn.addEventListener('click', () => this.simulateAlert());
        }
        if (this.alertHistoryPanelClose) {
            this.alertHistoryPanelClose.addEventListener('click', () => this.hideAlertPanel('history'));
        }
        if (this.alertDetailPanelClose) {
            this.alertDetailPanelClose.addEventListener('click', () => this.hideAlertPanel('detail'));
        }
        if (this.knowledgePanelBtn) {
            this.knowledgePanelBtn.addEventListener('click', () => this.showKnowledgePanel());
        }
        if (this.knowledgePanelClose) {
            this.knowledgePanelClose.addEventListener('click', () => this.hideKnowledgePanel());
        }

        // 主题切换
        if (this.themeToggleBtn) {
            this.themeToggleBtn.addEventListener('click', () => this.toggleTheme());
        }

        // 点击面板外部关闭
        document.addEventListener('click', (e) => {
            if (this.alertHistoryPanel && this.alertHistoryPanel.classList.contains('open')) {
                if (!e.target.closest('.alert-panel') && !e.target.closest('#alertHistoryBtn')) {
                    this.hideAlertPanel('history');
                }
            }
            if (this.alertDetailPanel && this.alertDetailPanel.classList.contains('open')) {
                if (!e.target.closest('.alert-detail-panel')) {
                    this.hideAlertPanel('detail');
                }
            }
            if (this.knowledgePanel && this.knowledgePanel.classList.contains('open')) {
                if (!e.target.closest('#knowledgePanel') && !e.target.closest('#knowledgePanelBtn')) {
                    this.hideKnowledgePanel();
                }
            }
        });
    }

    // ==================== 侧边栏 (移动端) ====================

    setupMobileSidebar() {
        this.mobileMenuBtn = document.createElement('button');
        this.mobileMenuBtn.className = 'mobile-menu-btn';
        this.mobileMenuBtn.innerHTML = `
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                <path d="M3 6H21M3 12H21M3 18H21" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
            </svg>
        `;
        this.mobileMenuBtn.setAttribute('aria-label', '菜单');
        document.body.appendChild(this.mobileMenuBtn);

        this.mobileMenuBtn.addEventListener('click', () => this.toggleMobileSidebar(true));
        if (this.sidebarBackdrop) {
            this.sidebarBackdrop.addEventListener('click', () => this.toggleMobileSidebar(false));
        }
    }

    toggleMobileSidebar(open) {
        if (!this.sidebar || !this.sidebarBackdrop) return;
        if (window.innerWidth > 768) return;
        this.sidebar.classList.toggle('open', open);
        this.sidebarBackdrop.classList.toggle('show', open);
    }

    // ==================== 暗黑模式 ====================

    initTheme() {
        const savedTheme = localStorage.getItem('theme');
        if (savedTheme === 'dark') {
            document.documentElement.setAttribute('data-theme', 'dark');
            this.updateThemeIcons(true);
        } else if (savedTheme === 'light') {
            document.documentElement.setAttribute('data-theme', 'light');
            this.updateThemeIcons(false);
        } else {
            // 跟随系统
            document.documentElement.removeAttribute('data-theme');
            const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
            this.updateThemeIcons(isDark);
        }
        this.updateThemeColor();
    }

    toggleTheme() {
        const currentTheme = document.documentElement.getAttribute('data-theme');
        let isDark;
        if (!currentTheme) {
            // 当前跟随系统，切换到 explicit 模式
            const systemDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
            if (systemDark) {
                document.documentElement.setAttribute('data-theme', 'light');
                localStorage.setItem('theme', 'light');
                isDark = false;
            } else {
                document.documentElement.setAttribute('data-theme', 'dark');
                localStorage.setItem('theme', 'dark');
                isDark = true;
            }
        } else if (currentTheme === 'dark') {
            document.documentElement.setAttribute('data-theme', 'light');
            localStorage.setItem('theme', 'light');
            isDark = false;
        } else {
            // Explicit light → 跟随系统
            document.documentElement.removeAttribute('data-theme');
            localStorage.removeItem('theme');
            isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
        }
        this.updateThemeIcons(isDark);
        this.updateThemeColor();
        this.updateHljsTheme(isDark);
    }

    updateThemeIcons(isDark) {
        if (this.themeIconSun && this.themeIconMoon) {
            this.themeIconSun.style.display = isDark ? 'block' : 'none';
            this.themeIconMoon.style.display = isDark ? 'none' : 'block';
        }
    }

    updateThemeColor() {
        const meta = document.getElementById('themeColorMeta');
        if (meta) {
            const isDark = document.documentElement.getAttribute('data-theme') === 'dark' ||
                (!document.documentElement.getAttribute('data-theme') &&
                 window.matchMedia('(prefers-color-scheme: dark)').matches);
            meta.setAttribute('content', isDark ? '#0B1120' : '#1E3A5F');
        }
    }

    updateHljsTheme(isDark) {
        const hljsLink = document.getElementById('hljsTheme');
        if (hljsLink) {
            hljsLink.href = isDark
                ? 'https://cdn.jsdelivr.net/npm/highlight.js@11.9.0/styles/github-dark.min.css'
                : 'https://cdn.jsdelivr.net/npm/highlight.js@11.9.0/styles/github.min.css';
        }
    }

    // ==================== 工具菜单 ====================

    toggleToolsMenu() {
        if (this.toolsMenu && this.toolsBtn) {
            const wrapper = this.toolsBtn.closest('.tools-btn-wrapper');
            if (wrapper) wrapper.classList.toggle('active');
        }
    }

    closeToolsMenu() {
        if (this.toolsMenu && this.toolsBtn) {
            const wrapper = this.toolsBtn.closest('.tools-btn-wrapper');
            if (wrapper) wrapper.classList.remove('active');
        }
    }

    // ==================== 对话管理 ====================

    newChat() {
        if (this.isStreaming) {
            this.showNotification('请等待当前对话完成后再新建对话', 'warning');
            return;
        }

        // 如果当前有对话内容，保存
        if (this.currentChatHistory.length > 0) {
            if (this.isCurrentChatFromHistory) {
                this.updateCurrentChatHistory();
            } else {
                this.saveCurrentChat();
            }
        }

        this.isStreaming = false;
        if (this.messageInput) this.messageInput.value = '';
        this.currentChatHistory = [];
        this.isCurrentChatFromHistory = false;
        if (this.chatMessages) this.chatMessages.innerHTML = '';
        this.sessionId = this.generateSessionId();
        this.currentMode = 'quick';
        this.updateUI();
        this.checkAndSetCentered();
        this.renderChatHistory();
    }

    saveCurrentChat() {
        if (this.currentChatHistory.length === 0) return;

        const existingIndex = this.chatHistories.findIndex(h => h.id === this.sessionId);
        if (existingIndex !== -1) {
            this.updateCurrentChatHistory();
            return;
        }

        const firstUserMessage = this.currentChatHistory.find(msg => msg.type === 'user');
        const title = firstUserMessage
            ? (firstUserMessage.content.substring(0, 30) + (firstUserMessage.content.length > 30 ? '...' : ''))
            : '新对话';

        this.chatHistories.unshift({
            id: this.sessionId,
            title: title,
            messages: [...this.currentChatHistory],
            createdAt: new Date().toISOString(),
            updatedAt: new Date().toISOString()
        });

        if (this.chatHistories.length > 50) {
            this.chatHistories = this.chatHistories.slice(0, 50);
        }
        this.saveChatHistories();
    }

    updateCurrentChatHistory() {
        if (this.currentChatHistory.length === 0) return;

        const existingIndex = this.chatHistories.findIndex(h => h.id === this.sessionId);
        if (existingIndex === -1) {
            this.saveCurrentChat();
            return;
        }

        const history = this.chatHistories[existingIndex];
        history.messages = [...this.currentChatHistory];
        history.updatedAt = new Date().toISOString();

        const firstUserMessage = this.currentChatHistory.find(msg => msg.type === 'user');
        if (firstUserMessage) {
            const newTitle = firstUserMessage.content.substring(0, 30) +
                (firstUserMessage.content.length > 30 ? '...' : '');
            if (history.title !== newTitle) history.title = newTitle;
        }
        this.saveChatHistories();
    }

    loadChatHistories() {
        try {
            const stored = localStorage.getItem('chatHistories');
            return stored ? JSON.parse(stored) : [];
        } catch (e) {
            return [];
        }
    }

    async loadServerChatHistories() {
        try {
            const response = await fetch(`${this.apiBaseUrl}/chat/sessions`);
            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            const payload = await response.json();
            const serverHistories = (payload.data || []).map(session => {
                const local = this.chatHistories.find(history => history.id === session.sessionId);
                return {
                    id: session.sessionId,
                    title: session.title || '新对话',
                    messages: local ? local.messages : null,
                    createdAt: session.createTime ? new Date(session.createTime).toISOString() : new Date().toISOString(),
                    updatedAt: session.updateTime ? new Date(session.updateTime).toISOString() : new Date().toISOString(),
                    serverBacked: true
                };
            });

            const serverIds = new Set(serverHistories.map(history => history.id));
            const localOnly = this.chatHistories.filter(history => !serverIds.has(history.id));
            this.chatHistories = [...serverHistories, ...localOnly];
            this.saveChatHistories();
            this.renderChatHistory();
        } catch (error) {
            console.warn('加载服务端历史对话失败，使用本地缓存:', error);
        }
    }

    saveChatHistories() {
        try {
            localStorage.setItem('chatHistories', JSON.stringify(this.chatHistories));
        } catch (e) {
            console.error('保存历史对话失败:', e);
        }
    }

    renderChatHistory() {
        if (!this.chatHistoryList) return;
        this.chatHistoryList.innerHTML = '';
        if (this.chatHistories.length === 0) return;

        this.chatHistories.forEach((history) => {
            const historyItem = document.createElement('div');
            historyItem.className = 'history-item';
            historyItem.dataset.historyId = history.id;
            historyItem.innerHTML = `
                <div class="history-item-content">
                    <span class="history-item-title">${this.escapeHtml(history.title)}</span>
                </div>
                <button class="history-item-delete" data-history-id="${history.id}" title="删除">
                    <svg viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                        <path d="M18 6L6 18M6 6L18 18" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>
                    </svg>
                </button>
            `;

            historyItem.addEventListener('click', (e) => {
                if (!e.target.closest('.history-item-delete')) {
                    this.loadChatHistory(history.id);
                    this.toggleMobileSidebar(false);
                }
            });

            const deleteBtn = historyItem.querySelector('.history-item-delete');
            deleteBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                this.deleteChatHistory(history.id);
            });

            this.chatHistoryList.appendChild(historyItem);
        });
    }

    async loadChatHistory(historyId) {
        const history = this.chatHistories.find(h => h.id === historyId);
        if (!history) return;

        if (this.currentChatHistory.length > 0 && this.sessionId !== historyId) {
            if (this.isCurrentChatFromHistory) {
                this.updateCurrentChatHistory();
            } else {
                this.saveCurrentChat();
            }
        }

        if (!history.messages || history.serverBacked) {
            try {
                history.messages = await this.fetchChatHistoryMessages(historyId);
                this.saveChatHistories();
            } catch (error) {
                console.warn('加载服务端对话消息失败，使用本地缓存:', error);
                if (!history.messages) {
                    this.showNotification('加载历史消息失败: ' + error.message, 'error');
                    return;
                }
            }
        }

        this.sessionId = history.id;
        this.currentChatHistory = [...history.messages];
        this.isCurrentChatFromHistory = true;

        if (this.chatMessages) {
            this.chatMessages.innerHTML = '';
            history.messages.forEach(msg => {
                this.addMessage(msg.type, msg.content, false, false);
            });
        }
        this.checkAndSetCentered();
        this.renderChatHistory();
    }

    async fetchChatHistoryMessages(historyId) {
        const response = await fetch(`${this.apiBaseUrl}/chat/session/${encodeURIComponent(historyId)}/messages`);
        if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);
        const payload = await response.json();
        if (payload.code !== 200 || !payload.data) {
            throw new Error(payload.message || '会话不存在');
        }
        return (payload.data.messageHistory || []).map(message => ({
            type: message.role === 'assistant' ? 'assistant' : 'user',
            content: message.content || '',
            timestamp: new Date().toISOString()
        }));
    }

    async deleteChatHistory(historyId) {
        try {
            const response = await fetch(`${this.apiBaseUrl}/chat/session/${encodeURIComponent(historyId)}`, {
                method: 'DELETE'
            });
            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);
        } catch (error) {
            console.warn('删除服务端历史对话失败，仅删除本地缓存:', error);
        }

        this.chatHistories = this.chatHistories.filter(h => h.id !== historyId);
        this.saveChatHistories();
        this.renderChatHistory();

        if (this.sessionId === historyId) {
            this.currentChatHistory = [];
            if (this.chatMessages) this.chatMessages.innerHTML = '';
            this.sessionId = this.generateSessionId();
            this.checkAndSetCentered();
        }
    }

    // ==================== 模式切换 ====================

    toggleModeDropdown() {
        if (this.modeSelectorBtn && this.modeDropdown) {
            const wrapper = this.modeSelectorBtn.closest('.mode-selector-wrapper');
            if (wrapper) wrapper.classList.toggle('active');
        }
    }

    closeModeDropdown() {
        if (this.modeSelectorBtn && this.modeDropdown) {
            const wrapper = this.modeSelectorBtn.closest('.mode-selector-wrapper');
            if (wrapper) wrapper.classList.remove('active');
        }
    }

    selectMode(mode) {
        if (this.isStreaming) {
            this.showNotification('请等待当前对话完成后再切换模式', 'warning');
            return;
        }
        this.currentMode = mode;
        this.updateUI();
        const modeNames = { 'quick': '快速', 'stream': '流式' };
        this.showNotification(`已切换到${modeNames[mode]}模式`, 'info');
    }

    updateUI() {
        if (this.currentModeText) {
            const modeNames = { 'quick': '快速', 'stream': '流式' };
            this.currentModeText.textContent = modeNames[this.currentMode] || '快速';
        }

        document.querySelectorAll('.dropdown-item').forEach(item => {
            const mode = item.getAttribute('data-mode');
            item.classList.toggle('active', mode === this.currentMode);
        });

        if (this.sendButton) this.sendButton.disabled = this.isStreaming;
        if (this.messageInput) {
            this.messageInput.disabled = this.isStreaming;
        }
    }

    generateSessionId() {
        return 'session_' + Math.random().toString(36).substr(2, 9) + '_' + Date.now();
    }

    // ==================== 消息发送 ====================

    async sendMessage() {
        let message = '';
        if (this.messageInput) {
            message = this.messageInput.value.trim();
        }
        if (!message) {
            this.showNotification('请输入消息内容', 'warning');
            return;
        }
        if (this.isStreaming) {
            this.showNotification('请等待当前对话完成', 'warning');
            return;
        }

        this.addMessage('user', message);
        if (this.messageInput) {
            this.messageInput.value = '';
            this.autoResizeInput();
        }

        this.isStreaming = true;
        this.updateUI();

        try {
            if (this.currentMode === 'quick') {
                await this.sendQuickMessage(message);
            } else if (this.currentMode === 'stream') {
                await this.sendStreamMessage(message);
            }
        } catch (error) {
            console.error('发送消息失败:', error);
            this.addMessage('assistant', '抱歉，发送消息时出现错误：' + error.message);
        } finally {
            this.isStreaming = false;
            this.updateUI();
            if (this.currentChatHistory.length > 0) {
                if (this.isCurrentChatFromHistory) {
                    this.updateCurrentChatHistory();
                } else {
                    this.saveCurrentChat();
                }
                this.renderChatHistory();
                this.loadServerChatHistories();
            }
        }
    }

    async sendQuickMessage(message) {
        const loadingMessage = this.addLoadingMessage('正在思考...');

        try {
            const response = await fetch(`${this.apiBaseUrl}/chat`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ Id: this.sessionId, Question: message })
            });

            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            const data = await response.json();

            if (loadingMessage && loadingMessage.parentNode) {
                loadingMessage.parentNode.removeChild(loadingMessage);
            }

            if (data.code === 200 || data.message === 'success') {
                const chatResponse = data.data;
                if (chatResponse && chatResponse.success) {
                    this.addMessage('assistant', chatResponse.answer || '（无回复内容）');
                } else if (chatResponse && chatResponse.errorMessage) {
                    throw new Error(chatResponse.errorMessage);
                } else {
                    this.addMessage('assistant', chatResponse?.answer || chatResponse?.errorMessage || '服务返回了空内容');
                }
            } else {
                throw new Error(data.message || '请求失败');
            }
        } catch (error) {
            if (loadingMessage && loadingMessage.parentNode) {
                loadingMessage.parentNode.removeChild(loadingMessage);
            }
            throw error;
        }
    }

    async sendStreamMessage(message) {
        try {
            const response = await fetch(`${this.apiBaseUrl}/chat_stream`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ Id: this.sessionId, Question: message })
            });

            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            const assistantMessageElement = this.addMessage('assistant', '', true);
            let fullResponse = '';

            const reader = response.body.getReader();
            const decoder = new TextDecoder();
            let buffer = '';
            let currentEvent = '';

            try {
                while (true) {
                    const { done, value } = await reader.read();

                    if (done) {
                        this.handleStreamComplete(assistantMessageElement, fullResponse);
                        break;
                    }

                    buffer += decoder.decode(value, { stream: true });
                    const lines = buffer.split('\n');
                    buffer = lines.pop() || '';

                    for (const line of lines) {
                        if (line.trim() === '') continue;

                        if (line.startsWith('id:')) continue;
                        if (line.startsWith('event:')) {
                            currentEvent = line.substring(6).trim();
                            continue;
                        }
                        if (!line.startsWith('data:')) continue;

                        const rawData = line.substring(5).trim();

                        if (rawData === '[DONE]') {
                            this.handleStreamComplete(assistantMessageElement, fullResponse);
                            return;
                        }

                        try {
                            const sseMessage = JSON.parse(rawData);
                            if (sseMessage && typeof sseMessage.type === 'string') {
                                if (sseMessage.type === 'content') {
                                    fullResponse += sseMessage.data || '';
                                    if (assistantMessageElement) {
                                        const messageContent = assistantMessageElement.querySelector('.message-content');
                                        messageContent.innerHTML = this.renderMarkdown(fullResponse);
                                        this.highlightCodeBlocks(messageContent);
                                        this.scrollToBottom();
                                    }
                                } else if (sseMessage.type === 'done') {
                                    this.handleStreamComplete(assistantMessageElement, fullResponse);
                                    return;
                                } else if (sseMessage.type === 'error') {
                                    if (assistantMessageElement) {
                                        const messageContent = assistantMessageElement.querySelector('.message-content');
                                        messageContent.innerHTML = this.renderMarkdown('错误: ' + (sseMessage.data || '未知错误'));
                                    }
                                    return;
                                }
                            } else {
                                fullResponse += rawData;
                                if (assistantMessageElement) {
                                    const messageContent = assistantMessageElement.querySelector('.message-content');
                                    messageContent.innerHTML = this.renderMarkdown(fullResponse);
                                    this.highlightCodeBlocks(messageContent);
                                    this.scrollToBottom();
                                }
                            }
                        } catch (e) {
                            fullResponse += rawData;
                            if (assistantMessageElement) {
                                const messageContent = assistantMessageElement.querySelector('.message-content');
                                messageContent.innerHTML = this.renderMarkdown(fullResponse);
                                this.highlightCodeBlocks(messageContent);
                                this.scrollToBottom();
                            }
                        }
                    }
                }
            } finally {
                reader.releaseLock();
            }
        } catch (error) {
            throw error;
        }
    }

    // ==================== 消息渲染 ====================

    addMessage(type, content, isStreaming = false, saveToHistory = true) {
        const isFirstMessage = this.chatMessages &&
            this.chatMessages.querySelectorAll('.message').length === 0;

        if (!isStreaming && saveToHistory && content) {
            this.currentChatHistory.push({
                type: type,
                content: content,
                timestamp: new Date().toISOString()
            });
        }

        const messageDiv = document.createElement('div');
        messageDiv.className = `message ${type}${isStreaming ? ' streaming' : ''}`;

        if (type === 'assistant') {
            const messageAvatar = document.createElement('div');
            messageAvatar.className = 'message-avatar';
            messageAvatar.innerHTML = `
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                    <path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" fill="white"/>
                </svg>
            `;
            messageDiv.appendChild(messageAvatar);
        }

        const messageContentWrapper = document.createElement('div');
        messageContentWrapper.className = 'message-content-wrapper';

        const messageContent = document.createElement('div');
        messageContent.className = 'message-content';

        if (type === 'assistant' && !isStreaming) {
            messageContent.innerHTML = this.renderMarkdown(content);
            this.highlightCodeBlocks(messageContent);
        } else {
            messageContent.textContent = content;
        }

        messageContentWrapper.appendChild(messageContent);
        messageDiv.appendChild(messageContentWrapper);

        if (this.chatMessages) {
            this.chatMessages.appendChild(messageDiv);
            if (isFirstMessage && this.chatContainer) {
                this.chatContainer.classList.remove('centered');
            }
            this.scrollToBottom();
        }

        return messageDiv;
    }

    addLoadingMessage(content) {
        const messageDiv = document.createElement('div');
        messageDiv.className = 'message assistant';

        const messageAvatar = document.createElement('div');
        messageAvatar.className = 'message-avatar';
        messageAvatar.innerHTML = `
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                <path d="M12 2L15.09 8.26L22 9.27L17 14.14L18.18 21.02L12 17.77L5.82 21.02L7 14.14L2 9.27L8.91 8.26L12 2Z" fill="white"/>
            </svg>
        `;
        messageDiv.appendChild(messageAvatar);

        const messageContentWrapper = document.createElement('div');
        messageContentWrapper.className = 'message-content-wrapper';

        const messageContent = document.createElement('div');
        messageContent.className = 'message-content loading-message-content';
        messageContent.innerHTML = `
            <span>${this.escapeHtml(content)}</span>
            <span class="loading-spinner-icon">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
                    <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8z" fill="currentColor" opacity="0.2"/>
                    <path d="M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10c1.54 0 3-.36 4.28-1l-1.5-2.6C13.64 19.62 12.84 20 12 20c-4.41 0-8-3.59-8-8s3.59-8 8-8c.84 0 1.64.38 2.18 1l1.5-2.6C13 2.36 12.54 2 12 2z" fill="currentColor"/>
                </svg>
            </span>
        `;

        messageContentWrapper.appendChild(messageContent);
        messageDiv.appendChild(messageContentWrapper);

        if (this.chatMessages) {
            this.chatMessages.appendChild(messageDiv);
            const isFirstMessage = this.chatMessages.querySelectorAll('.message').length === 1;
            if (isFirstMessage && this.chatContainer) {
                this.chatContainer.classList.remove('centered');
            }
            this.scrollToBottom();
        }

        return messageDiv;
    }

    checkAndSetCentered() {
        if (this.chatMessages && this.chatContainer) {
            const hasMessages = this.chatMessages.querySelectorAll('.message').length > 0;
            this.chatContainer.classList.toggle('centered', !hasMessages);
        }
    }

    scrollToBottom() {
        if (this.chatMessages) {
            this.chatMessages.scrollTop = this.chatMessages.scrollHeight;
        }
    }

    handleStreamComplete(assistantMessageElement, fullResponse) {
        if (assistantMessageElement) {
            assistantMessageElement.classList.remove('streaming');
            const messageContent = assistantMessageElement.querySelector('.message-content');
            if (messageContent) {
                messageContent.innerHTML = this.renderMarkdown(fullResponse);
                this.highlightCodeBlocks(messageContent);
            }
        }
        if (fullResponse) {
            this.currentChatHistory.push({
                type: 'assistant',
                content: fullResponse,
                timestamp: new Date().toISOString()
            });
            if (this.isCurrentChatFromHistory) {
                this.updateCurrentChatHistory();
                this.renderChatHistory();
            }
        }
    }

    autoResizeInput() {
        if (this.messageInput) {
            this.messageInput.style.height = 'auto';
            this.messageInput.style.height = Math.min(this.messageInput.scrollHeight, 150) + 'px';
        }
    }

    // ==================== 通知 ====================

    showNotification(message, type = 'info') {
        const notification = document.createElement('div');
        notification.className = `notification ${type}`;
        notification.textContent = message;

        const colors = {
            info: 'var(--color-info)',
            success: 'var(--color-success)',
            warning: 'var(--color-warning)',
            error: 'var(--color-error)'
        };
        notification.style.backgroundColor = colors[type] || colors.info;

        document.body.appendChild(notification);
        setTimeout(() => {
            notification.style.animation = 'slideOut 0.3s ease';
            setTimeout(() => {
                if (notification.parentNode) {
                    notification.parentNode.removeChild(notification);
                }
            }, 300);
        }, 3000);
    }

    // ==================== 文件上传 ====================

    handleFileSelect(event) {
        const file = event.target.files[0];
        if (file) {
            if (!this.validateFileType(file)) {
                this.showNotification('只支持上传 TXT 或 Markdown (.md) 格式的文件', 'error');
                this.fileInput.value = '';
                return;
            }
            this.uploadFile(file);
        }
    }

    validateFileType(file) {
        const fileName = file.name.toLowerCase();
        return ['.txt', '.md', '.markdown'].some(ext => fileName.endsWith(ext));
    }

    async uploadFile(file) {
        if (!this.validateFileType(file)) {
            this.showNotification('只支持上传 TXT 或 Markdown (.md) 格式的文件', 'error');
            return;
        }

        const maxSize = 50 * 1024 * 1024;
        if (file.size > maxSize) {
            this.showNotification('文件大小不能超过50MB', 'error');
            return;
        }

        this.isStreaming = true;
        this.updateUI();
        this.showOverlay(true, '正在上传文件...', `上传: ${file.name}`);

        try {
            const formData = new FormData();
            formData.append('file', file);

            const response = await fetch(`${this.apiBaseUrl}/upload`, {
                method: 'POST',
                body: formData
            });

            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            const data = await response.json();

            if ((data.code === 200 || data.message === 'success') && data.data) {
                const indexStatus = data.data.indexStatus || 'INDEXING';
                const indexTaskId = data.data.indexTaskId || '-';
                const message = data.data.message || '文件已接收，索引处理中';
                this.addMessage('assistant',
                    `${file.name} ${message}\n\n索引任务: \`${indexTaskId}\`\n当前状态: \`${indexStatus}\``,
                    false,
                    true);
                if (this.knowledgePanel && this.knowledgePanel.classList.contains('open')) {
                    this.loadKnowledgeIndexTasks();
                }
            } else {
                throw new Error(data.message || '上传失败');
            }
        } catch (error) {
            console.error('文件上传失败:', error);
            this.showNotification('文件上传失败: ' + error.message, 'error');
        } finally {
            if (this.fileInput) this.fileInput.value = '';
            this.isStreaming = false;
            this.showOverlay(false);
            this.updateUI();
        }
    }

    formatFileSize(bytes) {
        if (bytes === 0) return '0 Bytes';
        const k = 1024;
        const sizes = ['Bytes', 'KB', 'MB', 'GB'];
        const i = Math.floor(Math.log(bytes) / Math.log(k));
        return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i];
    }

    // ==================== 知识库 ====================

    hideKnowledgePanel() {
        if (this.knowledgePanel) {
            this.knowledgePanel.style.display = 'none';
            this.knowledgePanel.classList.remove('open');
        }
    }

    async showKnowledgePanel() {
        if (!this.knowledgePanel || !this.knowledgePanelContent) return;

        this.knowledgePanel.style.display = 'flex';
        this.knowledgePanel.classList.add('open');
        this.knowledgePanelContent.innerHTML = this.renderKnowledgePanelShell();
        this.bindKnowledgePanelEvents();

        await Promise.all([
            this.loadDependencies(),
            this.loadKnowledgeIndexTasks()
        ]);
    }

    renderKnowledgePanelShell() {
        return `
            <div class="knowledge-search">
                <div class="knowledge-search-row">
                    <input id="knowledgeSearchInput" class="knowledge-search-input" type="search" placeholder="测试知识库检索" maxlength="200">
                    <select id="knowledgeTopKSelect" class="knowledge-topk-select" aria-label="返回条数">
                        <option value="3">Top 3</option>
                        <option value="5" selected>Top 5</option>
                        <option value="10">Top 10</option>
                    </select>
                    <button id="knowledgeSearchBtn" class="knowledge-search-btn">检索</button>
                </div>
                <div id="knowledgeSearchResult" class="knowledge-search-result">
                    <div class="knowledge-muted">输入关键词后可查看过滤条件、粗排参数和命中文档。</div>
                </div>
            </div>
            <div class="knowledge-section-block">
                <div class="knowledge-section-title">依赖状态</div>
                <div id="dependencyStatusList" class="dependency-status-list">
                    <div class="alert-panel-loading">加载中...</div>
                </div>
            </div>
            <div class="knowledge-section-block">
                <div class="knowledge-section-title">索引任务</div>
                <div id="knowledgeIndexTasks" class="knowledge-task-list">
                    <div class="alert-panel-loading">加载中...</div>
                </div>
            </div>
        `;
    }

    bindKnowledgePanelEvents() {
        const searchInput = document.getElementById('knowledgeSearchInput');
        const searchBtn = document.getElementById('knowledgeSearchBtn');

        if (searchBtn) {
            searchBtn.addEventListener('click', () => this.searchKnowledge());
        }
        if (searchInput) {
            searchInput.addEventListener('keydown', (e) => {
                if (e.key === 'Enter') {
                    e.preventDefault();
                    this.searchKnowledge();
                }
            });
            setTimeout(() => searchInput.focus(), 0);
        }
    }

    async loadDependencies() {
        const dependencyContainer = document.getElementById('dependencyStatusList');
        if (!dependencyContainer) return;

        this.dependencyLoadError = '';
        try {
            const response = await fetch(`${this.apiBaseUrl}/system/dependencies`);
            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            const payload = await response.json();
            this.dependencies = Array.isArray(payload.data) ? payload.data : [];
            this.renderDependencyStatus();
        } catch (error) {
            console.error('获取依赖状态失败:', error);
            this.dependencies = [];
            this.dependencyLoadError = error.message || '未知错误';
            this.renderDependencyStatus();
        }
    }

    renderDependencyStatus() {
        const dependencyContainer = document.getElementById('dependencyStatusList');
        if (!dependencyContainer) return;

        if (this.dependencyLoadError) {
            dependencyContainer.innerHTML =
                '<div class="alert-panel-error">获取依赖状态失败: ' + this.escapeHtml(this.dependencyLoadError) + '</div>';
            return;
        }

        if (!this.dependencies || this.dependencies.length === 0) {
            dependencyContainer.innerHTML = '<div class="alert-panel-empty">暂无依赖状态</div>';
            return;
        }

        dependencyContainer.innerHTML = this.dependencies.map(dependency => {
            const name = dependency.name || dependency.dependencyName || dependency.id || '未知依赖';
            const state = dependency.state || dependency.status || 'UNKNOWN';
            const stateClass = this.dependencyStateClass(state);
            const lastError = dependency.lastError || '';
            const openedAtText = dependency.openedAt ? new Date(dependency.openedAt).toLocaleString('zh-CN') : '';
            const failureRate = Number.isFinite(Number(dependency.failureRate)) ? Number(dependency.failureRate).toFixed(1) + '%' : '';
            const slowCallRate = Number.isFinite(Number(dependency.slowCallRate)) ? Number(dependency.slowCallRate).toFixed(1) + '%' : '';
            const bufferedCalls = Number.isFinite(Number(dependency.bufferedCalls)) ? String(dependency.bufferedCalls) : '';
            const metaParts = [
                lastError ? `最近错误: ${this.escapeHtml(lastError)}` : '',
                openedAtText ? `打开时间: ${this.escapeHtml(openedAtText)}` : '',
                failureRate ? `失败率: ${this.escapeHtml(failureRate)}` : '',
                slowCallRate ? `慢调用率: ${this.escapeHtml(slowCallRate)}` : '',
                bufferedCalls ? `样本数: ${this.escapeHtml(bufferedCalls)}` : ''
            ].filter(Boolean);
            return `
                <div class="dependency-status-item ${stateClass}">
                    <div class="dependency-status-main">
                        <span class="dependency-status-name">${this.escapeHtml(name)}</span>
                        <span class="dependency-status-badge ${stateClass}">${this.escapeHtml(state)}</span>
                    </div>
                    ${metaParts.length > 0 ? `<div class="dependency-status-meta">${metaParts.join(' · ')}</div>` : ''}
                </div>
            `;
        }).join('');
    }

    dependencyStateClass(state) {
        const normalized = String(state || '').toUpperCase();
        if (normalized === 'OPEN') return 'dependency-state-open';
        if (normalized === 'HALF_OPEN') return 'dependency-state-half-open';
        if (normalized === 'CLOSED') return 'dependency-state-closed';
        if (normalized === 'DISABLED') return 'dependency-state-disabled';
        return 'dependency-state-unknown';
    }

    async loadKnowledgeIndexTasks() {
        const tasksContainer = document.getElementById('knowledgeIndexTasks');
        if (!tasksContainer) return;

        try {
            const response = await fetch(`${this.apiBaseUrl}/knowledge/index-tasks`);
            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            const payload = await response.json();
            this.renderKnowledgeIndexTasks(payload.data || []);
        } catch (error) {
            console.error('获取索引任务失败:', error);
            tasksContainer.innerHTML =
                '<div class="alert-panel-error">获取索引任务失败: ' + this.escapeHtml(error.message) + '</div>';
        }
    }

    renderKnowledgeIndexTasks(tasks) {
        const tasksContainer = document.getElementById('knowledgeIndexTasks');
        if (!tasksContainer) return;

        if (!tasks || tasks.length === 0) {
            tasksContainer.innerHTML = '<div class="alert-panel-empty">暂无索引任务</div>';
            return;
        }

        const html = tasks.map(task => {
            const status = task.status || 'UNKNOWN';
            const statusClass = this.knowledgeStatusClass(status);
            const updatedAt = task.updatedAt ? new Date(task.updatedAt).toLocaleString('zh-CN') : '未知';
            const errorHtml = task.errorMessage
                ? `<div class="knowledge-task-error">${this.escapeHtml(task.errorMessage)}</div>`
                : '';
            return `
                <div class="knowledge-task-card">
                    <div class="knowledge-task-header">
                        <span class="knowledge-task-name">${this.escapeHtml(task.fileName || '未知文件')}</span>
                        <span class="knowledge-task-status ${statusClass}">${this.escapeHtml(status)}</span>
                    </div>
                    <div class="knowledge-task-meta">${this.escapeHtml(task.message || '')}</div>
                    <div class="knowledge-task-meta">更新时间: ${this.escapeHtml(updatedAt)}</div>
                    ${errorHtml}
                </div>
            `;
        }).join('');

        tasksContainer.innerHTML = html;
    }

    knowledgeStatusClass(status) {
        const normalized = status.toLowerCase();
        if (normalized === 'completed') return 'status-completed';
        if (normalized === 'failed') return 'status-failed';
        return 'status-indexing';
    }

    async searchKnowledge() {
        const input = document.getElementById('knowledgeSearchInput');
        const topKSelect = document.getElementById('knowledgeTopKSelect');
        const resultContainer = document.getElementById('knowledgeSearchResult');
        if (!input || !resultContainer) return;

        const query = input.value.trim();
        if (!query) {
            this.showNotification('请输入知识库检索关键词', 'warning');
            return;
        }

        const topK = topKSelect ? topKSelect.value : '5';
        resultContainer.innerHTML = '<div class="alert-panel-loading">检索中...</div>';

        try {
            const params = new URLSearchParams({ query, topK });
            const response = await fetch(`${this.apiBaseUrl}/knowledge/search?${params.toString()}`);
            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            const payload = await response.json();
            this.renderKnowledgeSearchResult(payload.data);
        } catch (error) {
            console.error('知识库检索失败:', error);
            resultContainer.innerHTML =
                '<div class="alert-panel-error">知识库检索失败: ' + this.escapeHtml(error.message) + '</div>';
        }
    }

    renderKnowledgeSearchResult(trace) {
        const resultContainer = document.getElementById('knowledgeSearchResult');
        if (!resultContainer) return;

        if (!trace) {
            resultContainer.innerHTML = '<div class="alert-panel-empty">暂无检索结果</div>';
            return;
        }

        const results = trace.results || [];
        const paramsHtml = `
            <div class="knowledge-trace-grid">
                <div><strong>TopK</strong><span>${this.escapeHtml(String(trace.requestedTopK || '-'))}</span></div>
                <div><strong>CandidateK</strong><span>${this.escapeHtml(String(trace.searchK || '-'))}</span></div>
                <div><strong>HNSW ef</strong><span>${this.escapeHtml(String(trace.searchEf || '-'))}</span></div>
            </div>
            <div class="knowledge-filter">
                <strong>过滤条件</strong>
                <code>${this.escapeHtml(trace.filterExpr || '-')}</code>
            </div>
        `;

        if (results.length === 0) {
            resultContainer.innerHTML = paramsHtml + '<div class="alert-panel-empty">未命中文档</div>';
            return;
        }

        const resultsHtml = results.map((result, index) => {
            const metadata = this.formatKnowledgeMetadata(result.metadata);
            const score = Number.isFinite(result.score) ? result.score.toFixed(4) : String(result.score || '-');
            return `
                <div class="knowledge-result-card">
                    <div class="knowledge-result-header">
                        <span class="knowledge-result-rank">#${index + 1}</span>
                        <span class="knowledge-result-score">score ${this.escapeHtml(score)}</span>
                    </div>
                    <div class="knowledge-result-content">${this.escapeHtml(result.content || '')}</div>
                    ${metadata ? `<div class="knowledge-result-meta">${metadata}</div>` : ''}
                </div>
            `;
        }).join('');

        resultContainer.innerHTML = paramsHtml + '<div class="knowledge-result-list">' + resultsHtml + '</div>';
    }

    formatKnowledgeMetadata(metadata) {
        if (!metadata) return '';
        try {
            const parsed = typeof metadata === 'string' ? JSON.parse(metadata) : metadata;
            return Object.entries(parsed)
                .map(([key, value]) => `<span>${this.escapeHtml(key)}: ${this.escapeHtml(String(value))}</span>`)
                .join('');
        } catch (e) {
            return `<span>${this.escapeHtml(metadata)}</span>`;
        }
    }

    // ==================== 加载遮罩 ====================

    showOverlay(show, text, subtext) {
        if (!this.loadingOverlay) return;
        if (show) {
            this.loadingOverlay.classList.add('show');
            if (this.loadingText && text) this.loadingText.textContent = text;
            if (this.loadingSubtext && subtext) this.loadingSubtext.textContent = subtext;
            document.body.style.overflow = 'hidden';
        } else {
            this.loadingOverlay.classList.remove('show');
            document.body.style.overflow = '';
        }
    }

    // ==================== AI Ops ====================

    parseSSEJson(data) {
        // 支持单JSON对象和多个JSON对象（拼接在同一行）
        const results = [];
        const jsonPattern = /\{"type"\s*:\s*"[^"]+"\s*,\s*"data"\s*:\s*(?:"[^"]*"|null)\}/g;
        const matches = data.match(jsonPattern);

        if (matches && matches.length > 0) {
            for (const jsonStr of matches) {
                try {
                    results.push(JSON.parse(jsonStr));
                } catch (e) {
                    // 跳过解析失败的
                }
            }
        } else {
            try {
                results.push(JSON.parse(data));
            } catch (e) {
                return null; // 非JSON格式
            }
        }
        return results.length > 0 ? results : null;
    }

    async sendAIOpsRequest(loadingMessageElement, alertContext, alertId) {
        const body = (alertContext || alertId)
            ? JSON.stringify({ alertContext: alertContext || '', alertId: alertId || '' })
            : '{}';

        try {
            const response = await fetch(`${this.apiBaseUrl}/ai_ops`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: body
            });

            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            let fullResponse = '';
            const reader = response.body.getReader();
            const decoder = new TextDecoder();
            let buffer = '';

            try {
                while (true) {
                    const { done, value } = await reader.read();
                    if (done) {
                        if (fullResponse) {
                            this.updateAIOpsMessage(loadingMessageElement, fullResponse, []);
                        }
                        break;
                    }

                    buffer += decoder.decode(value, { stream: true });
                    const lines = buffer.split('\n');
                    buffer = lines.pop() || '';

                    for (const line of lines) {
                        if (line.trim() === '') continue;
                        if (line.startsWith('id:')) continue;
                        if (line.startsWith('event:')) continue;
                        if (!line.startsWith('data:')) continue;

                        const rawData = line.substring(5).trim();
                        const parsedMessages = this.parseSSEJson(rawData);

                        if (parsedMessages === null) {
                            // 非JSON格式，直接追加
                            fullResponse += rawData;
                            this.updateAIOpsStreamContent(loadingMessageElement, fullResponse);
                            continue;
                        }

                        let isDone = false;
                        for (const msg of parsedMessages) {
                            if (msg.type === 'content') {
                                fullResponse += msg.data || '';
                            } else if (msg.type === 'done') {
                                isDone = true;
                                this.updateAIOpsMessage(loadingMessageElement, fullResponse, []);
                            } else if (msg.type === 'error') {
                                throw new Error(msg.data || '智能运维分析失败');
                            }
                        }

                        if (!isDone && loadingMessageElement) {
                            this.updateAIOpsStreamContent(loadingMessageElement, fullResponse);
                        }
                        if (isDone) return;
                    }
                }
            } finally {
                reader.releaseLock();
            }
        } catch (error) {
            throw error;
        }
    }

    updateAIOpsStreamContent(messageElement, content) {
        if (!messageElement) return;
        messageElement.classList.add('aiops-message');

        const wrapper = messageElement.querySelector('.message-content-wrapper');
        if (!wrapper) return;

        let messageContent = wrapper.querySelector('.message-content');
        if (!messageContent) {
            messageContent = document.createElement('div');
            messageContent.className = 'message-content';
            wrapper.appendChild(messageContent);
        }
        messageContent.textContent = content;
        this.scrollToBottom();
    }

    updateAIOpsMessage(messageElement, response) {
        if (!messageElement) return;

        messageElement.classList.add('aiops-message');
        const wrapper = messageElement.querySelector('.message-content-wrapper');
        if (!wrapper) return;

        const messageContent = wrapper.querySelector('.message-content');
        if (!messageContent) return;

        messageContent.classList.remove('loading-message-content');
        // 移除加载图标
        const loadingIcon = messageContent.querySelector('.loading-spinner-icon');
        if (loadingIcon) loadingIcon.remove();
        // 保留文字节点，但移除旧的文字
        const textSpan = messageContent.querySelector('span');
        if (textSpan) textSpan.remove();

        // 渲染Markdown
        messageContent.innerHTML = this.renderMarkdown(response);
        this.highlightCodeBlocks(messageContent);

        this.currentChatHistory.push({
            type: 'assistant',
            content: response,
            timestamp: new Date().toISOString()
        });

        this.scrollToBottom();
        return messageElement;
    }

    // ==================== 告警系统 ====================

    connectAlertSSE() {
        // 关闭旧连接
        if (this.alertEventSource) {
            this.alertEventSource.close();
        }

        this.alertEventSource = new EventSource(`${this.apiBaseUrl}/alerts/stream`);

        this.alertEventSource.onmessage = (event) => {
            try {
                const data = JSON.parse(event.data);
                if (data.type === 'new_alert') {
                    this.showAlertNotification(data);
                }
            } catch (e) {
                console.error('解析告警SSE数据失败:', e);
            }
        };

        this.alertEventSource.onerror = () => {
            console.error('告警SSE连接断开，5秒后重试');
            if (this.alertEventSource) {
                this.alertEventSource.close();
                this.alertEventSource = null;
            }
            setTimeout(() => this.connectAlertSSE(), 5000);
        };
    }

    showAlertNotification(data) {
        this.pendingAlertId = data.incidentId || data.alertId;

        // 移除旧通知
        const oldNotif = document.getElementById('_dynamic_alert_notif');
        if (oldNotif) oldNotif.remove();

        const isFiring = data.status === 'firing';
        const color = isFiring ? '#d93025' : '#f9ab00';
        const notif = document.createElement('div');
        notif.id = '_dynamic_alert_notif';
        notif.className = 'alert-notification';

        notif.innerHTML = `
            <div class="alert-notification-header" style="background:${color}">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M18 8C18 4.68629 15.3137 2 12 2C8.68629 2 6 4.68629 6 8V12L4 15H20L18 12V8Z" stroke-linecap="round" stroke-linejoin="round"/>
                    <path d="M9 17V18C9 19.6569 10.3431 21 12 21C13.6569 21 15 19.6569 15 18V17" stroke-linecap="round" stroke-linejoin="round"/>
                </svg>
                <span class="alert-notification-title">新告警</span>
                <span class="alert-notification-close" id="_alert_notif_close">&times;</span>
            </div>
            <div class="alert-notification-body">
                ${this.escapeHtml(data.summary || '收到新的告警推送')}
            </div>
            <div class="alert-notification-footer">
                <button class="alert-notification-btn" id="_alert_notif_view">查看详情</button>
            </div>
        `;

        document.body.appendChild(notif);

        // 使用addEventListener代替内联onclick
        const closeBtn = notif.querySelector('#_alert_notif_close');
        if (closeBtn) {
            closeBtn.addEventListener('click', () => notif.remove());
        }

        const viewBtn = notif.querySelector('#_alert_notif_view');
        if (viewBtn) {
            viewBtn.addEventListener('click', () => {
                notif.remove();
                if (data.incidentId) {
                    this.showAlertDetail(this.pendingAlertId);
                } else if (this.pendingAlertId) {
                    this.showAlertHistory();
                }
            });
        }

        // 10秒后自动移除
        setTimeout(() => {
            const el = document.getElementById('_dynamic_alert_notif');
            if (el) el.remove();
        }, 10000);
    }

    hideAlertPanel(type) {
        const panel = type === 'history' ? this.alertHistoryPanel : this.alertDetailPanel;
        if (type === 'detail') {
            clearTimeout(this.incidentDetailRefreshTimer);
            this.incidentDetailRefreshTimer = null;
            this.incidentDetailRenderSignature = null;
        }
        if (panel) {
            panel.style.display = 'none';
            panel.classList.remove('open');
        }
    }

    async showAlertHistory() {
        if (!this.alertHistoryPanel || !this.alertHistoryContent) return;

        this.alertHistoryPanel.style.display = 'flex';
        this.alertHistoryPanel.classList.add('open');
        this.alertHistoryContent.innerHTML = '<div class="alert-panel-loading">加载中...</div>';
        this.alertHistoryContent.scrollTop = 0;

        try {
            const response = await fetch(`${this.apiBaseUrl}/incidents`);
            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            const data = await response.json();
            this.renderAlertHistory(data.data || []);
        } catch (error) {
            console.error('获取事故历史失败:', error);
            this.alertHistoryContent.innerHTML =
                '<div class="alert-panel-error">获取事故历史失败: ' + this.escapeHtml(error.message) + '</div>';
        }
    }

    renderAlertHistory(incidents) {
        if (!this.alertHistoryContent) return;

        if (!incidents || incidents.length === 0) {
            this.alertHistoryContent.innerHTML = '<div class="alert-panel-empty">暂无事故记录</div>';
            return;
        }

        let html = '';
        incidents.forEach(incident => {
            const severityClass = incident.severity === 'critical' ? 'severity-critical' : 'severity-warning';
            const time = new Date(incident.updatedAt || incident.lastAlertAt || incident.createdAt).toLocaleString('zh-CN');
            const statusText = this.incidentStatusText(incident.status);
            const statusClass = incident.status === 'RESOLVED' ? 'status-resolved' : 'status-firing';
            const runText = incident.latestRunStatus
                ? this.runStatusText(incident.latestRunStatus)
                : '尚未诊断';

            html += `
                <div class="alert-card" data-incident-id="${this.escapeHtml(incident.id)}">
                    <div class="alert-card-header">
                        <span class="alert-severity ${severityClass}">${this.escapeHtml(incident.severity || 'unknown')}</span>
                        <span class="alert-status ${statusClass}">${statusText}</span>
                        <span class="alert-time">${time}</span>
                    </div>
                    <div class="alert-card-body">${this.escapeHtml(incident.title || '未知事故')}</div>
                    <div class="alert-card-footer">
                        <span class="${incident.latestRunStatus === 'COMPLETED' ? 'alert-has-report' : 'alert-no-report'}">
                            ${this.escapeHtml(runText)} · ${incident.alertCount || 0} 次告警
                        </span>
                        <button class="alert-view-btn" data-incident-id="${this.escapeHtml(incident.id)}">查看详情</button>
                    </div>
                </div>
            `;
        });

        this.alertHistoryContent.innerHTML = '<div class="alert-card-list">' + html + '</div>';

        this.alertHistoryContent.querySelectorAll('.alert-view-btn').forEach(btn => {
            btn.addEventListener('click', (e) => {
                e.stopPropagation();
                const incidentId = btn.getAttribute('data-incident-id');
                if (incidentId) {
                    this.hideAlertPanel('history');
                    this.showAlertDetail(incidentId);
                }
            });
        });
    }

    async showAlertDetail(incidentId, options = {}) {
        if (!this.alertDetailPanel || !this.alertDetailContent) return;

        const silent = options.silent === true;
        const preserveScroll = options.preserveScroll === true;
        const previousScrollTop = preserveScroll ? this.alertDetailContent.scrollTop : 0;

        clearTimeout(this.incidentDetailRefreshTimer);
        this.incidentDetailRefreshTimer = null;

        if (!silent) {
            this.incidentDetailRenderSignature = null;
            this.alertDetailPanel.style.display = 'flex';
            this.alertDetailPanel.classList.add('open');
            this.alertDetailContent.innerHTML = '<div class="alert-panel-loading">加载中...</div>';
            this.alertDetailContent.scrollTop = 0;
        } else if (!this.alertDetailPanel.classList.contains('open')) {
            return;
        }

        try {
            const detailResponse = await fetch(`${this.apiBaseUrl}/incidents/${incidentId}`);
            if (!detailResponse.ok) throw new Error(`获取事故详情失败: ${detailResponse.status}`);
            const responseData = await detailResponse.json();
            const detailData = responseData.data;
            const renderSignature = this.buildIncidentDetailRenderSignature(detailData);
            const preRuns = detailData.diagnosisRuns || [];
            const preLatestRun = preRuns.length > 0 ? preRuns[preRuns.length - 1] : null;
            if (silent && renderSignature === this.incidentDetailRenderSignature) {
                this.scheduleIncidentDetailRefresh(incidentId, preLatestRun);
                return;
            }
            this.incidentDetailRenderSignature = renderSignature;

            const time = new Date(detailData.lastAlertAt || detailData.updatedAt || detailData.createdAt).toLocaleString('zh-CN');
            let alertsHtml = '';
            const payloads = detailData.alertPayloads || [];

            payloads.forEach((payload, payloadIndex) => {
                (payload.alerts || []).forEach(alert => {
                    const labels = alert.labels ? this.formatKeyValue(alert.labels) : '无';
                    const annotations = alert.annotations ? this.formatKeyValue(alert.annotations) : '无';
                    alertsHtml += `
                        <div class="alert-detail-item">
                            <div><strong>告警批次:</strong> #${payloadIndex + 1}</div>
                            <div><strong>状态:</strong> ${this.escapeHtml(alert.status || 'unknown')}</div>
                            <div><strong>开始时间:</strong> ${this.escapeHtml(alert.startsAt || '未知')}</div>
                            <div><strong>Fingerprint:</strong> ${this.escapeHtml(alert.fingerprint || '无')}</div>
                            <div><strong>标签:</strong> ${this.escapeHtml(labels)}</div>
                            <div><strong>注解:</strong> ${this.escapeHtml(annotations)}</div>
                        </div>
                    `;
                });
            });

            const runs = detailData.diagnosisRuns || [];
            const latestRun = runs.length > 0 ? runs[runs.length - 1] : null;
            let reportHtml = '<div class="alert-panel-empty">暂无诊断任务</div>';
            let runHtml = '<div class="alert-panel-empty">暂无诊断记录</div>';
            let evidenceHtml = '<div class="alert-panel-empty">暂无工具证据</div>';

            if (latestRun) {
                const runTime = latestRun.completedAt || latestRun.startedAt || latestRun.createdAt;
                const evidence = latestRun.evidence || [];
                const toolEvidence = evidence.filter(item => this.isToolEvidence(item));
                runHtml = `
                    <div class="alert-detail-info">
                        <div><strong>Run ID:</strong> ${this.escapeHtml(latestRun.runId)}</div>
                        <div><strong>状态:</strong> ${this.escapeHtml(this.runStatusText(latestRun.status))}</div>
                        <div><strong>更新时间:</strong> ${runTime ? new Date(runTime).toLocaleString('zh-CN') : '未知'}</div>
                        ${latestRun.currentStep ? `<div><strong>当前步骤:</strong> ${this.escapeHtml(latestRun.currentStep)}</div>` : ''}
                        ${latestRun.currentTool ? `<div><strong>当前工具:</strong> ${this.escapeHtml(latestRun.currentTool)}</div>` : ''}
                        ${latestRun.progressMessage ? `<div><strong>进度:</strong> ${this.escapeHtml(latestRun.progressMessage)}</div>` : ''}
                        <div><strong>工具证据数量:</strong> ${toolEvidence.length} 条</div>
                        ${latestRun.errorMessage ? `<div><strong>失败原因:</strong> ${this.escapeHtml(latestRun.errorMessage)}</div>` : ''}
                    </div>
                `;

                evidenceHtml = this.renderDiagnosisEvidenceList(toolEvidence);

                if (latestRun.status === 'COMPLETED' && latestRun.report) {
                    reportHtml = this.renderMarkdown(latestRun.report);
                } else if (latestRun.status === 'FAILED') {
                    reportHtml = '<div class="alert-panel-error">' + this.escapeHtml(latestRun.errorMessage || '诊断失败') + '</div>';
                } else {
                    reportHtml = '<div class="alert-panel-empty">诊断任务正在执行，请稍后刷新详情</div>';
                }

                if (['QUEUED', 'RUNNING', 'WAITING_TOOL'].includes(latestRun.status)) {
                    this.scheduleIncidentDetailRefresh(incidentId, latestRun);
                }
            }

            this.alertDetailContent.innerHTML = `
                <div class="alert-detail-section">
                    <div class="alert-detail-info">
                        <div><strong>Incident ID:</strong> ${this.escapeHtml(detailData.id)}</div>
                        <div><strong>标题:</strong> ${this.escapeHtml(detailData.title || '未知事故')}</div>
                        <div><strong>状态:</strong> ${this.escapeHtml(this.incidentStatusText(detailData.status))}</div>
                        <div><strong>级别:</strong> ${this.escapeHtml(detailData.severity || 'unknown')}</div>
                        <div><strong>累计告警:</strong> ${detailData.alertCount || 0} 次</div>
                        <div><strong>最近告警:</strong> ${time}</div>
                    </div>
                </div>
                <div class="alert-detail-section">
                    <h4>关联告警</h4>
                    ${alertsHtml || '<div class="alert-panel-empty">无告警数据</div>'}
                </div>
                <div class="alert-detail-section">
                    <h4>最新诊断</h4>
                    ${runHtml}
                </div>
                <div class="alert-detail-section">
                    <h4>工具证据</h4>
                    <div class="diagnosis-evidence-list">${evidenceHtml}</div>
                </div>
                <div class="alert-detail-section">
                    <h4>分析报告</h4>
                    <div class="alert-report-content">${reportHtml}</div>
                    <div class="alert-detail-actions">
                        <button class="alert-aiops-btn" data-incident-id="${this.escapeHtml(incidentId)}">重新执行 AI Ops 诊断</button>
                        ${latestRun && latestRun.status === 'COMPLETED' ? `<button class="alert-archive-case-btn" data-incident-id="${this.escapeHtml(incidentId)}">写入历史案例</button>` : ''}
                    </div>
                </div>
            `;

            const aiOpsBtn = this.alertDetailContent.querySelector('.alert-aiops-btn');
            if (aiOpsBtn) {
                aiOpsBtn.addEventListener('click', () => {
                    const id = aiOpsBtn.getAttribute('data-incident-id');
                    if (id) {
                        this.triggerIncidentDiagnosis(id);
                    }
                });
            }
            const archiveCaseBtn = this.alertDetailContent.querySelector('.alert-archive-case-btn');
            if (archiveCaseBtn) {
                archiveCaseBtn.addEventListener('click', () => {
                    const id = archiveCaseBtn.getAttribute('data-incident-id');
                    if (id) {
                        this.archiveIncidentCase(id);
                    }
                });
            }
            if (preserveScroll) {
                this.alertDetailContent.scrollTop = previousScrollTop;
            }

        } catch (error) {
            console.error('获取事故详情失败:', error);
            if (silent) {
                return;
            }
            this.alertDetailContent.innerHTML =
                '<div class="alert-panel-error">获取事故详情失败: ' + this.escapeHtml(error.message) + '</div>';
        }
    }

    async simulateAlert() {
        try {
            const response = await fetch(`${this.apiBaseUrl}/alerts/simulate`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: '{}'
            });

            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);

            const data = await response.json();
            if (!data.success) {
                throw new Error(data.message || '模拟告警失败');
            }
            this.showNotification('模拟告警已触发', 'success');
            if (data.incidentId) {
                this.showAlertDetail(data.incidentId);
            }
        } catch (error) {
            console.error('模拟告警失败:', error);
            this.showNotification('模拟告警失败: ' + error.message, 'error');
        }
    }

    async triggerIncidentDiagnosis(incidentId) {
        try {
            const response = await fetch(`${this.apiBaseUrl}/incidents/${incidentId}/diagnose`, {
                method: 'POST'
            });
            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);
            const data = await response.json();
            if (data.code !== 200) {
                throw new Error(data.message || '诊断任务提交失败');
            }
            this.showNotification('诊断任务已提交', 'success');
            this.showAlertDetail(incidentId);
            setTimeout(() => this.showAlertDetail(incidentId), 3000);
        } catch (error) {
            console.error('提交诊断任务失败:', error);
            this.showNotification('提交诊断任务失败: ' + error.message, 'error');
        }
    }

    async archiveIncidentCase(incidentId) {
        try {
            const response = await fetch(`${this.apiBaseUrl}/incidents/${incidentId}/archive-case`, {
                method: 'POST'
            });
            if (!response.ok) throw new Error(`HTTP错误: ${response.status}`);
            const data = await response.json();
            if (data.code !== 200) {
                throw new Error(data.message || '历史案例写入失败');
            }
            this.showNotification(data.data?.message || '历史案例已写入知识库', 'success');
        } catch (error) {
            console.error('历史案例写入失败:', error);
            this.showNotification('历史案例写入失败: ' + error.message, 'error');
        }
    }

    async triggerAIOps(alertId, alertContext) {
        if (this.isStreaming) {
            this.showNotification('请等待当前操作完成', 'warning');
            return;
        }

        this.newChat();
        this.isStreaming = true;
        this.updateUI();

        const loadingMessage = this.addLoadingMessage('分析中...');

        try {
            await this.sendAIOpsRequest(loadingMessage, alertContext, alertId);
        } catch (error) {
            console.error('智能运维分析失败:', error);
            if (loadingMessage) {
                const messageContent = loadingMessage.querySelector('.message-content');
                if (messageContent) {
                    messageContent.textContent = '抱歉，智能运维分析时出现错误：' + error.message;
                }
            }
        } finally {
            this.isStreaming = false;
            this.updateUI();
        }
    }

    // ==================== 工具 ====================

    formatKeyValue(values) {
        return Object.entries(values)
            .map(([key, value]) => `${key}: ${value}`)
            .join(', ');
    }

    isToolEvidence(item) {
        return !!item && (item.type === 'tool_call' || !!item.toolName);
    }

    renderDiagnosisEvidenceList(toolEvidence) {
        if (!toolEvidence || toolEvidence.length === 0) {
            return '<div class="alert-panel-empty">暂无工具证据</div>';
        }

        const highlights = [...toolEvidence]
            .sort((a, b) => this.evidenceImportanceScore(b) - this.evidenceImportanceScore(a))
            .slice(0, 4);
        const groups = this.groupDiagnosisEvidence(toolEvidence);

        const highlightHtml = highlights.map(item => this.renderDiagnosisEvidenceHighlight(item)).join('');
        const groupHtml = groups.map(group => {
            const preview = group.items
                .slice(0, 2)
                .map(item => item.summary || item.content || item.title || item.toolName || '无摘要')
                .map(text => this.escapeHtml(this.compactText(text, 48)))
                .join(' / ');
            const itemsHtml = group.items.map(item => this.renderDiagnosisEvidenceItem(item)).join('');
            return `
                <details class="diagnosis-evidence-group">
                    <summary class="diagnosis-evidence-group-summary">
                        <span class="diagnosis-evidence-group-title">${this.escapeHtml(group.label)}</span>
                        <span class="diagnosis-evidence-group-count">${group.items.length} 条</span>
                        ${preview ? `<span class="diagnosis-evidence-group-preview">${preview}</span>` : ''}
                    </summary>
                    <div class="diagnosis-evidence-group-body">
                        ${itemsHtml}
                    </div>
                </details>
            `;
        }).join('');

        return `
            <div class="diagnosis-evidence-overview">
                <div class="diagnosis-evidence-overview-title">关键证据摘要</div>
                <div class="diagnosis-evidence-highlight-list">${highlightHtml}</div>
            </div>
            <div class="diagnosis-evidence-groups">${groupHtml}</div>
        `;
    }

    groupDiagnosisEvidence(toolEvidence) {
        const groupDefinitions = [
            { key: 'cases', label: '相似历史案例' },
            { key: 'metrics', label: '指标趋势' },
            { key: 'logs', label: '日志查询' },
            { key: 'docs', label: '知识库检索' },
            { key: 'alerts', label: '活动告警' },
            { key: 'time', label: '时间工具' },
            { key: 'other', label: '其他工具' }
        ];
        const grouped = new Map(groupDefinitions.map(group => [group.key, { ...group, items: [] }]));
        toolEvidence.forEach(item => {
            const key = this.getDiagnosisEvidenceGroupKey(item);
            const group = grouped.get(key) || grouped.get('other');
            group.items.push(item);
        });
        return groupDefinitions
            .map(group => grouped.get(group.key))
            .filter(group => group.items.length > 0);
    }

    getDiagnosisEvidenceGroupKey(item) {
        const toolName = item && item.toolName ? item.toolName : '';
        if (toolName === 'searchSimilarIncidentCases') return 'cases';
        if (toolName === 'queryMetricTrend') return 'metrics';
        if (toolName === 'queryLogs') return 'logs';
        if (toolName === 'queryInternalDocs') return 'docs';
        if (toolName === 'queryPrometheusAlerts') return 'alerts';
        if (toolName.toLowerCase().includes('time') || toolName.toLowerCase().includes('date')) return 'time';
        return 'other';
    }

    evidenceImportanceScore(item) {
        if (!item) return 0;
        const summary = `${item.summary || ''} ${item.content || ''}`;
        let score = item.success === false ? 100 : 0;
        if (item.toolName === 'queryMetricTrend') {
            const payload = this.extractMetricTrendPayload(item);
            score += payload && payload.summary && payload.summary.anomalous === true ? 90 : 65;
        } else if (item.toolName === 'queryLogs') {
            score += 70;
        } else if (item.toolName === 'queryPrometheusAlerts') {
            score += 55;
        } else if (item.toolName === 'queryInternalDocs') {
            score += 40;
        }
        if (/critical|error|ERROR|OOM|OutOfMemory|异常|错误|失败|超时|突增|持续上升/i.test(summary)) {
            score += 25;
        }
        return score;
    }

    renderDiagnosisEvidenceHighlight(item) {
        const breakerOpen = item.errorCode === 'CIRCUIT_OPEN';
        const ok = item.success !== false && !breakerOpen;
        const summary = item.summary || item.content || '无摘要';
        const groupLabel = this.getDiagnosisEvidenceGroupLabel(item);
        return `
            <div class="diagnosis-evidence-highlight ${ok ? '' : 'failed'} ${breakerOpen ? 'breaker-open' : ''}">
                <div class="diagnosis-evidence-highlight-head">
                    <span>${this.escapeHtml(groupLabel)}</span>
                    <span>${this.escapeHtml(item.id || '-')}</span>
                </div>
                <div class="diagnosis-evidence-highlight-summary">${this.escapeHtml(this.compactText(summary, 120))}</div>
            </div>
        `;
    }

    getDiagnosisEvidenceGroupLabel(item) {
        const labels = {
            cases: '相似历史案例',
            metrics: '指标趋势',
            logs: '日志查询',
            docs: '知识库检索',
            alerts: '活动告警',
            time: '时间工具',
            other: '其他工具'
        };
        return labels[this.getDiagnosisEvidenceGroupKey(item)] || '工具证据';
    }

    renderDiagnosisEvidenceItem(item) {
        const breakerOpen = item.errorCode === 'CIRCUIT_OPEN';
        const ok = item.success !== false && !breakerOpen;
        const statusText = breakerOpen ? 'BREAKER OPEN' : (ok ? 'SUCCESS' : 'FAILED');
        const statusClass = breakerOpen ? 'failed breaker-open' : (ok ? 'success' : 'failed');
        const title = item.toolName || item.title || item.type || 'evidence';
        const summary = item.summary || item.content || '无摘要';
        return `
            <div class="diagnosis-evidence-item">
                <div class="diagnosis-evidence-head">
                    <div>
                        <div class="diagnosis-evidence-tool">${this.escapeHtml(title)}</div>
                        <div class="diagnosis-evidence-id">${this.escapeHtml(item.id || '-')}</div>
                    </div>
                    <span class="diagnosis-evidence-status ${statusClass}">${statusText}</span>
                </div>
                ${item.timeRange ? `<div class="diagnosis-evidence-time">时间范围: ${this.escapeHtml(item.timeRange)}</div>` : ''}
                <div class="diagnosis-evidence-summary">${this.escapeHtml(summary)}</div>
                ${item.errorCode ? `<div class="diagnosis-evidence-error-code">errorCode: ${this.escapeHtml(item.errorCode)}</div>` : ''}
                ${breakerOpen ? '<div class="diagnosis-evidence-breaker-note">依赖熔断，相关证据缺失；报告不能基于该工具生成事实结论。</div>' : ''}
                ${item.queryParams ? `
                    <details class="diagnosis-evidence-extra">
                        <summary>查看参数</summary>
                        <pre>${this.escapeHtml(item.queryParams)}</pre>
                    </details>
                ` : ''}
                ${this.renderMetricTrendChartDetails(item)}
            </div>
        `;
    }

    renderMetricTrendChartDetails(item) {
        const chart = this.renderMetricTrendChart(item);
        if (!chart) {
            return '';
        }
        return `
            <details class="diagnosis-evidence-extra diagnosis-evidence-chart-details">
                <summary>查看趋势图</summary>
                ${chart}
            </details>
        `;
    }

    extractMetricTrendPayload(item) {
        if (!item || item.toolName !== 'queryMetricTrend' || !item.rawFragment) {
            return null;
        }
        try {
            const payload = JSON.parse(item.rawFragment);
            const points = Array.isArray(payload.points)
                ? payload.points
                    .map(point => ({
                        timestamp: point.timestamp,
                        value: Number(point.value)
                    }))
                    .filter(point => Number.isFinite(point.value))
                : [];
            if (points.length < 2) {
                return null;
            }
            return {
                metric: payload.metric || 'metric',
                window: payload.window || item.timeRange || '',
                summary: payload.summary || {},
                points
            };
        } catch (error) {
            return null;
        }
    }

    renderMetricTrendChart(item) {
        const payload = this.extractMetricTrendPayload(item);
        if (!payload) {
            return '';
        }

        const width = 360;
        const height = 120;
        const padX = 14;
        const padTop = 14;
        const padBottom = 26;
        const chartWidth = width - padX * 2;
        const chartHeight = height - padTop - padBottom;
        const values = payload.points.map(point => point.value);
        const min = Math.min(...values);
        const max = Math.max(...values);
        const range = max - min || 1;
        const baseline = padTop + chartHeight;
        const path = payload.points.map((point, index) => {
            const x = padX + (index / (payload.points.length - 1)) * chartWidth;
            const y = padTop + ((max - point.value) / range) * chartHeight;
            return `${index === 0 ? 'M' : 'L'} ${x.toFixed(2)} ${y.toFixed(2)}`;
        }).join(' ');
        const firstX = padX;
        const lastX = padX + chartWidth;
        const areaPath = `${path} L ${lastX.toFixed(2)} ${baseline.toFixed(2)} L ${firstX.toFixed(2)} ${baseline.toFixed(2)} Z`;
        const latest = payload.summary.latest ?? values[values.length - 1];
        const avg = payload.summary.avg ?? (values.reduce((sum, value) => sum + value, 0) / values.length);
        const direction = payload.summary.direction || 'stable';
        const anomalous = payload.summary.anomalous === true;

        return `
            <div class="metric-trend-chart">
                <div class="metric-trend-header">
                    <div>
                        <div class="metric-trend-title">${this.escapeHtml(payload.metric)}</div>
                        <div class="metric-trend-subtitle">${this.escapeHtml(payload.window)} · ${payload.points.length} points</div>
                    </div>
                    <span class="metric-trend-badge ${anomalous ? 'anomalous' : 'normal'}">
                        ${anomalous ? '异常' : '正常'}
                    </span>
                </div>
                <svg class="trend-chart-svg" viewBox="0 0 ${width} ${height}" role="img" aria-label="${this.escapeHtml(payload.metric)} 趋势图">
                    <line x1="${padX}" y1="${padTop}" x2="${lastX}" y2="${padTop}" class="trend-chart-grid"></line>
                    <line x1="${padX}" y1="${baseline}" x2="${lastX}" y2="${baseline}" class="trend-chart-grid"></line>
                    <path d="${areaPath}" class="trend-chart-area"></path>
                    <path d="${path}" class="trend-chart-line"></path>
                    <circle cx="${lastX.toFixed(2)}" cy="${(padTop + ((max - values[values.length - 1]) / range) * chartHeight).toFixed(2)}" r="3.5" class="trend-chart-point"></circle>
                    <text x="${padX}" y="${height - 7}" class="trend-chart-label">min ${this.formatMetricValue(min)}</text>
                    <text x="${lastX}" y="${height - 7}" class="trend-chart-label trend-chart-label-end">latest ${this.formatMetricValue(latest)}</text>
                </svg>
                <div class="metric-trend-stats">
                    <span>max ${this.formatMetricValue(max)}</span>
                    <span>avg ${this.formatMetricValue(avg)}</span>
                    <span>${this.escapeHtml(this.metricDirectionText(direction))}</span>
                </div>
            </div>
        `;
    }

    formatMetricValue(value) {
        const number = Number(value);
        if (!Number.isFinite(number)) {
            return '-';
        }
        if (Math.abs(number) >= 100) {
            return number.toFixed(0);
        }
        if (Math.abs(number) >= 10) {
            return number.toFixed(1);
        }
        return number.toFixed(2);
    }

    metricDirectionText(direction) {
        const names = {
            increasing: '持续上升',
            decreasing: '持续下降',
            spiking: '突增',
            stable: '平稳'
        };
        return names[direction] || direction || '未知';
    }

    compactText(text, limit) {
        const value = String(text || '').replace(/\s+/g, ' ').trim();
        if (value.length <= limit) {
            return value;
        }
        return value.slice(0, Math.max(0, limit - 1)) + '…';
    }

    buildIncidentDetailRenderSignature(detailData) {
        const runs = detailData.diagnosisRuns || [];
        const latestRun = runs.length > 0 ? runs[runs.length - 1] : null;
        const toolEvidenceCount = latestRun
            ? (latestRun.evidence || []).filter(item => this.isToolEvidence(item)).length
            : 0;
        return JSON.stringify({
            id: detailData.id,
            updatedAt: detailData.updatedAt,
            lastAlertAt: detailData.lastAlertAt,
            alertCount: detailData.alertCount,
            runId: latestRun ? latestRun.runId : null,
            status: latestRun ? latestRun.status : null,
            currentStep: latestRun ? latestRun.currentStep : null,
            currentTool: latestRun ? latestRun.currentTool : null,
            progressMessage: latestRun ? latestRun.progressMessage : null,
            errorMessage: latestRun ? latestRun.errorMessage : null,
            completedAt: latestRun ? latestRun.completedAt : null,
            toolEvidenceCount,
            report: latestRun ? latestRun.report : null
        });
    }

    scheduleIncidentDetailRefresh(incidentId, latestRun) {
        clearTimeout(this.incidentDetailRefreshTimer);
        this.incidentDetailRefreshTimer = null;
        if (!latestRun || !['QUEUED', 'RUNNING', 'WAITING_TOOL'].includes(latestRun.status)) {
            return;
        }
        if (!this.alertDetailPanel || !this.alertDetailPanel.classList.contains('open')) {
            return;
        }
        this.incidentDetailRefreshTimer = setTimeout(
            () => this.showAlertDetail(incidentId, { silent: true, preserveScroll: true }),
            3000
        );
    }

    incidentStatusText(status) {
        const names = {
            OPEN: '处理中',
            RESOLVED: '已恢复'
        };
        return names[status] || status || '未知';
    }

    runStatusText(status) {
        const names = {
            QUEUED: '排队中',
            RUNNING: '诊断中',
            WAITING_TOOL: '等待工具返回',
            COMPLETED: '已完成',
            FAILED: '失败',
            CANCELLED: '已取消'
        };
        return names[status] || status || '未知';
    }

    escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }
}

// 初始化应用
document.addEventListener('DOMContentLoaded', () => {
    window.__app = new SuperBizAgentApp();
});
