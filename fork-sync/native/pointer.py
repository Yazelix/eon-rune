"""Small native virtual-pointer driver; only accepts this spike's Wayland socket.

Wire layout follows the locally installed wlr-virtual-pointer-unstable-v1.xml.
No input is sent through the system's input devices or the live display.
"""
import os, socket, struct, time
from pathlib import Path
W=os.environ["RUNE_SYNC_STATE"]+"/xdg"
def uints(*values): return struct.pack('='+'I'*len(values),*values)
def string(value):
    data=value.encode()+b'\0'
    return uints(len(data))+data+b'\0'*((-len(data))%4)
class Pointer:
    def __init__(self,env):
        assert env['XDG_RUNTIME_DIR']==W
        path=Path(W)/env['WAYLAND_DISPLAY']
        assert path.parent==Path(W) and path.is_socket()
        self.sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
        self.sock.settimeout(5);self.sock.connect(str(path));self.next=3;self.pending=b'';self.globals={}
        self.send(1,1,uints(2));self.sync()
        name,version=self.globals['zwlr_virtual_pointer_manager_v1']
        self.send(2,0,uints(name)+string('zwlr_virtual_pointer_manager_v1')+uints(min(version,2),4))
        self.send(4,0,uints(0,5));self.next=6;self.sync()
    def send(self,obj,op,data=b''):
        assert len(data)%4==0
        self.sock.sendall(uints(obj,((8+len(data))<<16)|op)+data)
    def event(self):
        while len(self.pending)<8:
            chunk=self.sock.recv(65536)
            assert chunk,'Wayland socket closed'
            self.pending+=chunk
        obj,word=struct.unpack('=II',self.pending[:8]);size=word>>16
        assert 8<=size<=65535 and size%4==0
        while len(self.pending)<size:
            chunk=self.sock.recv(65536)
            assert chunk,'Wayland socket closed'
            self.pending+=chunk
        body=self.pending[8:size];self.pending=self.pending[size:]
        return obj,word&65535,body
    def sync(self):
        callback=self.next;self.next+=1;self.send(1,0,uints(callback))
        while True:
            obj,op,data=self.event()
            if obj==1 and op==0: raise RuntimeError(f'Wayland protocol error: {data!r}')
            if obj==2 and op==0:
                name,length=struct.unpack('=II',data[:8]);interface=data[8:8+length-1].decode();offset=8+((length+3)//4)*4
                version=struct.unpack('=I',data[offset:offset+4])[0];self.globals[interface]=(name,version)
            if obj==callback: break
    def move(self,x,y):
        assert 0<=x<=2560 and 0<=y<=720
        self.send(5,1,uints(int(time.monotonic()*1000)&0xffffffff,int(x),int(y),2560,720));self.send(5,4);self.sync()
    def click(self,x,y,button=0x110):
        self.move(x,y);time.sleep(.15)
        for state in (1,0):
            self.send(5,2,uints(int(time.monotonic()*1000)&0xffffffff,button,state));self.send(5,4);self.sync();time.sleep(.1)
    def close(self): self.send(5,8);self.send(4,1);self.sock.close()
